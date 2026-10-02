#!/usr/bin/env python3
"""Enable missing detailed history only in the image built with LDK bd95e187.

Do not call this from the host pre-start hook: older pinned LDK images reject
the setting. Parse TOML, insert one line, then prove the sole semantic change.
Unusual TOML layouts are left intact for manual migration, never reserialized.
"""

import copy
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
import tomllib


KEY = "forwarded_payment_tracking_mode"
NODE_HEADER = re.compile(r"^[ \t]*\[[ \t]*(?:node|\"node\"|'node')[ \t]*\][ \t]*(?:#.*)?$")


class MigrationError(Exception):
    pass


def detailed_config(source):
    try:
        text = source.decode("utf-8")
        config = tomllib.loads(text)
    except (UnicodeError, tomllib.TOMLDecodeError):
        raise MigrationError("configuration is not valid UTF-8 TOML") from None
    node = config.get("node")
    if not isinstance(node, dict):
        raise MigrationError("configuration needs a [node] table")
    if KEY in node:
        return source  # Every explicit operator choice, including stats, wins.

    expected = copy.deepcopy(config)
    expected["node"][KEY] = "detailed"
    offset = 0
    candidates = []
    # A header-looking line can occur inside a multiline string. Only accept
    # an insertion when reparsing proves that no other value changed.
    for line in text.splitlines(keepends=True):
        offset += len(line)
        if not NODE_HEADER.fullmatch(line.rstrip("\r\n")):
            continue
        newline = "\r\n" if line.endswith("\r\n") else "\n"
        separator = "" if line.endswith("\n") else newline
        updated = text[:offset] + separator + f'{KEY} = "detailed"' + newline + text[offset:]
        try:
            if tomllib.loads(updated) == expected:
                candidates.append(updated.encode("utf-8"))
        except tomllib.TOMLDecodeError:
            continue
    if len(candidates) != 1:
        raise MigrationError("cannot safely insert into this [node] layout; edit it manually")
    return candidates[0]


def fingerprint(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns,
            info.st_ctime_ns, info.st_mode, info.st_uid, info.st_gid, info.st_nlink)


def migrate(path):
    path = Path(path)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
        raise MigrationError("configuration must be a regular, non-symlink file with one link")
    # O_NOFOLLOW also rejects a symlink substituted since lstat. NONBLOCK keeps
    # a substituted FIFO from hanging startup; fstat verifies the opened file.
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as source_file:
        if fingerprint(os.fstat(source_file.fileno())) != fingerprint(before):
            raise MigrationError("configuration changed during startup; stop concurrent edits")
        source = source_file.read()
        updated = detailed_config(source)
        if updated == source:
            return False
        # Preserve ACLs and other extended attributes as well as POSIX mode
        # and ownership. If the filesystem cannot do so, fail before replacing.
        attributes = {name: os.getxattr(source_file.fileno(), name)
                      for name in os.listxattr(source_file.fileno())}

    temp_fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.history-", dir=path.parent)
    try:
        with os.fdopen(temp_fd, "wb") as target:
            target.write(updated)
            target.flush()
            current = os.fstat(target.fileno())
            if (current.st_uid, current.st_gid) != (before.st_uid, before.st_gid):
                os.fchown(target.fileno(), before.st_uid, before.st_gid)
            for name in os.listxattr(target.fileno()):
                if name not in attributes:
                    os.removexattr(target.fileno(), name)
            os.fchmod(target.fileno(), stat.S_IMODE(before.st_mode))
            for name, value in attributes.items():
                os.setxattr(target.fileno(), name, value)
            os.fsync(target.fileno())
        if fingerprint(path.lstat()) != fingerprint(before):
            raise MigrationError("configuration changed during startup; stop concurrent edits")
        os.replace(temp_name, path)
    finally:
        # A failed write, metadata copy or rename leaves the original intact.
        if os.path.exists(temp_name):
            os.unlink(temp_name)
    return True


def main(argv):
    if len(argv) != 3:
        print("usage: ldk-server-entrypoint.py /usr/local/bin/ldk-server CONFIG.toml", file=sys.stderr)
        return 1
    try:
        changed = migrate(argv[2])
    except (MigrationError, OSError) as error:
        # Never include TOML or credentials in logs, even for malformed input.
        reason = str(error) if isinstance(error, MigrationError) else "file access or metadata preservation failed"
        print(f"LDK forwarded-history migration refused: {reason}. LDK Server was not started. "
              "Stop the app and back up its data. Validate the config and add "
              "forwarded_payment_tracking_mode = \"detailed\" under [node] manually for LDK bd95e187, "
              "or provide a writable directory mount with the existing file's ownership/permissions. "
              "See umbrel/README.md before retrying or downgrading.", file=sys.stderr)
        return 1
    if changed:
        print("LDK forwarded-history migration: enabled detailed mode (previously unset).", flush=True)
    # Replace PID 1 so SIGTERM reaches LDK's graceful-shutdown handler.
    os.execv(argv[1], argv[1:])


if __name__ == "__main__":
    sys.exit(main(sys.argv))
