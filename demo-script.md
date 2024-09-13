# Stable Channels Rust + LDK Demo

## Actors in this Demo
Each actor runs and Lightning Development Kit (LDK) Lightning Node.

- **Exchange**: Lightning-enabled exchange, like Coinbase or Kraken.
- **User**: This self-custodial user wants the USD stability, also known as the Stable Receiver.
- **LSP**: "Lightning Service Provider." This actor is the Stable Provider.

## Prerequisites:
1. Install Rust [LLM - add relevant URL].
2. Clone the repo:

git clone https://github.com/toneloc/stablechannels
cd stablechannels

## Walkthrough:

### Step 1 - Get Some Test BTC
Run the following commands to get your test Bitcoin addresses:

node1 getaddress
node3 getaddress

### Step 2 - Get Sats
Go to https://faucet.mutinynet.com/ and get some test sats.

### Step 3 - Open a Channel
Open a channel between the exchange and the LSP:

exchange openchannel

### Step 4 - Create a JIT Invoice
Create a JIT invoice that will route from the exchange, through the Lightning Service Provider, and finally to the user:

user getjitinvoice

### Step 5 - Pay the JIT Invoice
Pay the JIT invoice:

exchange payjitinvoice

### Step 6 - Stable Channels Demo
Start a stable channel using the command:

node1 startstablechannel cca0a4c065e678ad8aecec3ae9a6d694d1b5c7512290da69b32c72b6c209f6e2 true 4.0 0

## TODO:
- Move stable channels to...
