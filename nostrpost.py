import json
import os
import asyncio
from typing import List
from nostr.client.client import NostrClient
from dotenv import load_dotenv
load_dotenv()

async def main():
    print(f"Your nostr relays: {json.loads(os.getenv("NOSTR_RELAYS"))} ")
    client = NostrClient(
        private_key=os.getenv("NOSTR_PRIVATE_KEY"),
        relays=json.loads(os.getenv("NOSTR_RELAYS"))
    )
    print(f"Your nostr public key: {client.public_key.bech32()}")

    # Connect asynchronously
    client.connect()

    # Subscribe asynchronously
    client.subscribe()

    # Wait for 5 seconds
    await asyncio.sleep(10)

    # Publish event asynchronously (assuming publish_event is async)
    client.post('this is my message from Nostr library...')

if __name__ == "__main__":
    asyncio.run(main())

