import asyncio
from nostr.client.client import NostrClient

async def main():
    client = NostrClient(
        private_key='',
        relays=[
            "wss://wc1.current.ninja",
        ]
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

    await asyncio.sleep(10)

    # Publish event asynchronously (assuming publish_event is async)
    client.post('this is my message from Nostr library...')



if __name__ == "__main__":
    asyncio.run(main())

