#!/usr/bin/env python3
"""Offline entrypoint/hook tests; Python 3.11+, no Docker or network required."""

import importlib.util
import os
from pathlib import Path
import shutil
import stat
import struct
import subprocess
import tempfile
import tomllib
import unittest
from unittest import mock


UMBREL = Path(__file__).resolve().parents[1]
ENTRYPOINT = UMBREL / "docker/ldk-server-entrypoint.py"
HOOK = UMBREL / "stable-channels-lsp/hooks/pre-start"
spec = importlib.util.spec_from_file_location("entrypoint", ENTRYPOINT)
entrypoint = importlib.util.module_from_spec(spec)
spec.loader.exec_module(entrypoint)
KEY = entrypoint.KEY


class ForwardedHistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ldk-history-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.config = self.root / "ldk-server.toml"
        self.marker = self.root / "started"
        self.daemon = self.root / "fake-ldk-server"
        self.daemon.write_text(
            "#!/usr/bin/env python3\n"
            "import os, pathlib, sys, tomllib\n"
            "tomllib.loads(pathlib.Path(sys.argv[1]).read_text())\n"
            "pathlib.Path(os.environ['START_MARKER']).write_text(sys.argv[1])\n"
        )
        self.daemon.chmod(0o700)

    def write(self, text, mode=0o600):
        if self.config.exists():
            self.config.chmod(0o600)
        self.config.write_bytes(text.encode("utf-8"))
        self.config.chmod(mode)

    def start(self, path=None):
        return subprocess.run(
            ["python3", str(ENTRYPOINT), str(self.daemon), str(path or self.config)],
            env={**os.environ, "START_MARKER": str(self.marker)},
            capture_output=True, text=True, check=False,
        )

    def bootstrap(self, data):
        result = subprocess.run(
            ["bash", str(HOOK)],
            env={**os.environ, "APP_DATA_DIR": str(data), "APP_BITCOIN_NODE_IP": "bitcoin",
                 "APP_BITCOIN_RPC_USER": "rpc-user", "APP_BITCOIN_RPC_PASS": "private-secret",
                 "APP_BITCOIN_NETWORK": "regtest"},
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return data / "data/config/ldk-server.toml"

    def test_old_pinned_bootstrap_and_restart_remain_compatible(self):
        path = self.bootstrap(self.root / "app")
        before = path.read_bytes()
        self.assertNotIn(KEY, tomllib.loads(before.decode())["node"])
        self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
        path.chmod(0o640)
        private = self.root / "app/data/ldk-server/private-operator-data"
        private.write_text("keep me")
        private.chmod(0o400)
        config_info, private_info = path.stat(), private.stat()
        self.bootstrap(self.root / "app")
        self.assertEqual(path.read_bytes(), before)
        self.assertEqual(path.stat(), config_info)
        self.assertEqual(private.stat(), private_info)
        compose = (UMBREL / "stable-channels-lsp/docker-compose.yml").read_text()
        self.assertIn("sc-ldk-server:5f631bd@sha256:cd3e9cdea982fcd81e88f024bb4500da44001f542a5be22b962ec3459c980859", compose)
        self.assertNotIn("entrypoint:", compose)

    def test_new_image_migrates_fresh_config_before_starting(self):
        shutil.copytree(UMBREL / "stable-channels-lsp/data", self.root / "app/data")
        path = self.bootstrap(self.root / "app")
        before = tomllib.loads(path.read_text())
        result = self.start(path)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.marker.read_text(), str(path))
        before["node"][KEY] = "detailed"
        self.assertEqual(tomllib.loads(path.read_text()), before)
        self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_existing_config_preserves_bytes_comments_metadata_and_repeats(self):
        original = '# operator notes\r\n[bitcoind]\r\nrpc_password = "private-secret"\r\n\r\n  [ node ] # settings\r\nnetwork = "regtest"\r\n'
        self.write(original, 0o640)
        before = self.config.stat()
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        updated = self.config.read_bytes()
        self.assertEqual(updated, original.replace(
            '  [ node ] # settings\r\n',
            '  [ node ] # settings\r\nforwarded_payment_tracking_mode = "detailed"\r\n',
        ).encode())
        self.assertEqual((self.config.stat().st_uid, self.config.stat().st_gid), (before.st_uid, before.st_gid))
        self.assertEqual(stat.S_IMODE(self.config.stat().st_mode), 0o640)
        after = self.config.stat()
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.config.read_bytes(), updated)
        self.assertEqual(self.config.stat(), after)
        self.assertNotIn("private-secret", result.stdout + result.stderr)

    def test_explicit_modes_and_quoted_keys_are_preserved_without_write_access(self):
        for mode in ("stats", "detailed", "operator-future-value"):
            with self.subTest(mode=mode):
                self.write(f'["node"] # comment\n"{KEY}" = "{mode}" # choice\n', 0o400)
                original = self.config.read_bytes()
                before = self.config.stat()
                result = self.start()
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.config.read_bytes(), original)
                self.assertEqual(self.config.stat(), before)
                self.config.chmod(0o600)

    def test_header_inside_multiline_string_is_not_modified(self):
        original = 'notes = """\n[node]\nprivate-secret\n"""\n["node"] # real\nnetwork = "regtest"\n'
        self.write(original)
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.config.read_text(), original.replace(
            '["node"] # real\n', '["node"] # real\nforwarded_payment_tracking_mode = "detailed"\n',
        ))

    def test_no_final_newline(self):
        self.write("['node'] # last line")
        self.assertEqual(self.start().returncode, 0)
        self.assertEqual(tomllib.loads(self.config.read_text())["node"][KEY], "detailed")

    def test_commented_and_other_table_keys_do_not_mask_missing_node_key(self):
        original = f'[other]\n{KEY} = "stats"\n[node]\n# {KEY} = "stats"\nnetwork = "regtest"\n'
        self.write(original)
        self.assertEqual(self.start().returncode, 0)
        parsed = tomllib.loads(self.config.read_text())
        self.assertEqual(parsed["other"][KEY], "stats")
        self.assertEqual(parsed["node"][KEY], "detailed")
        self.assertIn(f'# {KEY} = "stats"', self.config.read_text())

    def test_malformed_or_unsupported_layout_never_starts_or_changes_source(self):
        for original in (
            '[node\nnetwork = "private-secret"\n',
            '[node]\nnetwork = "private-secret"\nnetwork = "duplicate"\n',
            '[node]\nforwarded_payment_tracking_mode = "stats"\n[broken\n',
            '[other]\nsecret = "private-secret"\n',
            'node = { network = "regtest" }\n',
            'node.network = "regtest"\n',
            '[[node]]\nnetwork = "regtest"\n',
        ):
            with self.subTest(original=original):
                self.write(original)
                before = self.config.stat()
                result = self.start()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.marker.exists())
                self.assertEqual(self.config.read_bytes(), original.encode())
                self.assertEqual(self.config.stat(), before)
                self.assertIn("LDK Server was not started", result.stderr)
                self.assertIn("back up", result.stderr)
                self.assertNotIn("private-secret", result.stdout + result.stderr)
                self.assertEqual(list(self.root.glob(".*.history-*")), [])

    def test_rename_or_metadata_failure_keeps_original_and_blocks_exec(self):
        self.write('[node]\nnetwork = "regtest"\n')
        original = self.config.read_bytes()
        for operation in ("replace", "fchmod", "fsync"):
            with self.subTest(operation=operation):
                with mock.patch.object(entrypoint.os, operation, side_effect=OSError("simulated failure")):
                    with mock.patch.object(entrypoint.os, "execv") as execute:
                        with mock.patch("sys.stderr"):
                            code = entrypoint.main([str(ENTRYPOINT), str(self.daemon), str(self.config)])
                        self.assertEqual(code, 1)
                        execute.assert_not_called()
                self.assertEqual(self.config.read_bytes(), original)
                self.assertEqual(list(self.root.glob(".*.history-*")), [])

    def test_readonly_directory_missing_key_fails_closed(self):
        if os.geteuid() == 0:
            self.skipTest("root bypasses directory write permission checks")
        self.write('[node]\nnetwork = "regtest"\n')
        original = self.config.read_bytes()
        self.root.chmod(0o500)
        try:
            result = self.start()
        finally:
            self.root.chmod(0o700)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.marker.exists())
        self.assertEqual(self.config.read_bytes(), original)

    def test_symlink_and_hardlink_sources_are_refused(self):
        target = self.root / "private-source"
        target.write_text('[node]\nnetwork = "regtest"\n')
        for link in (self.config.symlink_to, lambda p: os.link(p, self.config)):
            link(target)
            result = self.start()
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.marker.exists())
            self.assertNotIn(KEY, target.read_text())
            self.config.unlink()

    def test_extended_attributes_are_preserved(self):
        self.write('[node]\nnetwork = "regtest"\n')
        try:
            os.setxattr(self.config, "user.operator-note", b"private-metadata")
        except OSError:
            self.skipTest("temporary filesystem does not support user xattrs")
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(os.getxattr(self.config, "user.operator-note"), b"private-metadata")

    def test_posix_acl_is_preserved_without_inheriting_new_access(self):
        # Linux POSIX ACL xattr format: version, then (tag, perms, uid/gid).
        # Give a named user read access, masked to the group permission bits.
        acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", *entry) for entry in (
            (1, 6, 0xFFFFFFFF), (2, 4, 12345), (4, 0, 0xFFFFFFFF),
            (16, 4, 0xFFFFFFFF), (32, 0, 0xFFFFFFFF),
        ))
        self.write('[node]\nnetwork = "regtest"\n')
        try:
            os.setxattr(self.config, "system.posix_acl_access", acl)
        except OSError:
            self.skipTest("temporary filesystem does not support Linux POSIX ACLs")
        before = os.getxattr(self.config, "system.posix_acl_access")
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(os.getxattr(self.config, "system.posix_acl_access"), before)

        os.removexattr(self.config, "system.posix_acl_access")
        self.write('[node]\nnetwork = "regtest"\n', 0o640)
        os.setxattr(self.root, "system.posix_acl_default", acl)
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("system.posix_acl_access", os.listxattr(self.config))
        self.assertEqual(stat.S_IMODE(self.config.stat().st_mode), 0o640)


if __name__ == "__main__":
    unittest.main(verbosity=2)
