# Stable Channels + Rust + LDK + just-in-time channels

## Actors / roles in this demo

Each of these three actor runs a Lightning Development Kit (LDK) Lightning Node. 

Each actor remains self-custodial.

1. **Exchange**: Lightning-enabled exchange, like Coinbase or Kraken.
2. **User**: This self-custodial user wants the USD stability, also known as the Stable Receiver.
3. **LSP**: "Lightning Service Provider." This actor is the Stable Provider.

```mermaid
graph LR
    Exchange <---> LSP (Server) <---> User (Mobile)
```

## Prerequisites

To run this demo, you will need Rust installed. You must also be connected to the internet to use Mutinynet for testing.

Clone the repo and open it in two windows.

## Walkthrough

In this example, a user onboards to a Stable Channel from an exchange. 

The user onboards by paying himselg via a Bolt11 Lightning invoice. The LSP creates this channel for the user and provides this stabiltiy service.

## Step 1 - Start the app

- In one window, run:

 ```bash
 cargo run --features user
 ```

- In the other window, run:

 ```bash
 cargo run --features lsp
 ```

- In a third window, run:

 ```bash
 cargo run --features exchange
 ```


then 

``lsp getaddress``

and 

``exchange getaddress``

Go to https://faucet.mutinynet.com/ and send some test sats to these two addresses. Wait for them to confirm. 

``lsp balance``

and 

``exchange balance``

### Step 2 - Open a routing channel

Open a channel between the exchange and the LSP. We will use this for routing.

``exchange openchannel``

Let's see if the channel got confirmed on the blockchain. Check if "channel_ready" equals "true."

``lsp listallchannels``

or

``exchange listallchannels``

### Step 3 - Create a JIT Invoice

Create a JIT invoice that will route from the exchange, through the Lightning Service Provider, and finally to the user. 

``user getjitinvoice``

### Step 4 - Pay the JIT Invoice

The LSP intercepts the payment, takes out a channel open fee, puts in matching Liquidity, and sends the rest to the user.

``exchange payjitinvoice``

Now the LSP has two channels. 1 to the exchange and one to the user.

``lsp listallchannels``

And the user has one channel:

``user listallchannels``

### Step 5 - Start a stable channel 

Using the command:

``user startstablechannel CHANNEL_ID IS_STABLE_RECEIVER EXPECTED_DOLLAR_AMOUNT EXPECTED_BTC_AMOUNT``

or:

``user startstablechannel cca0a4c065e678ad8aecec3ae9a6d694d1b5c7512290da69b32c72b6c209f6e2 true 4.0 0``

