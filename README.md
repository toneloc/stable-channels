[![main on CLN v24.02.2](https://github.com/toneloc/stable-channels/actions/workflows/main_v24.02.yml/badge.svg?branch=main)](https://github.com/toneloc/stable-channels/actions/workflows/main_v24.02.yml)

## Stable Channels - Version 24SEP2024

Stable Channels allow Lightning Network node operators to maintain one side of a channel balance stable in dollar terms. The nodes receiving stability are called Stable Receivers, while the counterparts are Stable Providers, who assume the price volatility.

Each node queries five price feeds every minute. Based on the updated price, they adjust the channel balance with their counterparty to keep the Stable Receiver's balance at a fixed dollar value (e.g., $10,000 in bitcoin).

Both parties remain self-custodial and can opt out anytime via cooperative or forced on-chain channel closure. The project is in progress and compatible with LND, CLN, or LDK. The LND and CLN inmplmentations use Python; LDK uses Rust.

Links with examples:
- **Basics with example:** [Twitter thread](https://x.com/tonklaus/status/1729567459579945017)
- **In-depth discussion:** [Delving Bitcoin](https://delvingbitcoin.org/t/stable-channels-peer-to-peer-dollar-balances-on-lightning)
- **Project website:** [StableChannels.com](https://www.stablechannels.com)

### Developer Demo (LDK + Rust)

You will need Rust installed for this demo. You must also be connected to the internet to use Mutinynet for testing.

Clone the repo and open it in **two windows**.

#### Steps:

1. **Start up the app.**

   - In one window, run:

     ```bash
     cargo run --features user
     ```

   - In the other window, run:

     ```bash
     cargo run --features lsp
     ```

2. **Get some test BTC**

   - In the **user window**, run:

     ```bash
     getaddress
     ```

   - Go to [Mutinynet Faucet](https://faucet.mutinynet.com/) and send some test sats to the address you obtained.

   - In the **user window**, run:

     ```bash
     balance
     ```

     Wait until your BTC shows up there.

3. **Open the Stable Channel**

   - Open a channel by running in either window:

     ```bash
     openchannel [NODE_ID] [LISTENING_ADDRESS] [SATS_AMOUNT]
     ```

   - Then, run:

     ```bash
     listallchannels
     ```

     Check if `"channel_ready"` equals `"true"`. This will take 6 confirmations or a minute or two.

4. **Set Bolt12 Offers**

   - In **both windows**, run:

     ```bash
     getouroffer
     ```

   - Copy the offer from each window.

   - In **both windows**, run:

     ```bash
     settheiroffer [OFFER]
     ```

     Replace `[OFFER]` with the offer you copied from the other window.

5. **Start the Stable Channel for Both Users**

   | Window           | Command                                                                                                                          | Example Command                                                                                                                                        |
   |------------------|----------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------|
   | **User Window**  | `user startstablechannel CHANNEL_ID IS_STABLE_RECEIVER EXPECTED_DOLLAR_AMOUNT EXPECTED_BTC_AMOUNT`                                | `user startstablechannel cca0a... true 100.0 0`                                                                                                        |
   | **LSP Window**   | `user startstablechannel CHANNEL_ID IS_STABLE_RECEIVER EXPECTED_DOLLAR_AMOUNT EXPECTED_BTC_AMOUNT`                                | `user startstablechannel cca0a... false 100.0 0`                                                                                                       |

   - This command means:

     > Make the channel with ID `cca0a...` a stable channel with a value of $100.0 and 0 native bitcoin, where it is `true` (or `false`) that I am the stable receiver.

### Stable Channels Process

Every 1 minute, the price of bitcoin:

- **(a) Goes up:**
  - **Stable Receiver loses bitcoin.**
    - Less bitcoin is needed to maintain the dollar value.
    - The Stable Receiver pays the Stable Provider.
  
- **(b) Goes down:**
  - **Stable Receiver gains bitcoin.**
    - More bitcoin is needed to maintain the dollar value.
    - The Stable Provider pays the Stable Receiver.
  
- **(c) Stays the same:**
  - **No action required.**

*Note: Stable Channels are currently non-routing channels. Work is ongoing to add routing and payment capabilities.*

## Getting Started with LND and CLN

- **Supported Implementations:**
  - **CLN Plugin:** `stablechannels.py`
  - **Standalone LND App:** `lnd.py`
  - **Rust App (LDK):** Located in `src` (in development)
- **Additional Resources:** Explore `/platforms` for mobile apps, web apps, scripts, and servers.

There are also some in-progress web apps, bash scripts, Python servers and such Check it out in `/platforms`.

### Environment and dependencies

- **LDK Version:**
  - Requires Rust.
- **CLN and LND Versions:**
  - Requires Python 3.
  - **CLN Node:** Versions 23.05.2 or 24.02 recommended.
    - Version 23.08.1 is not supported.
  - **LND Node:** Tested with version 0.17.4-beta.
- **Dependencies Installation:**
  - Run `pip3 install -r requirements.txt` or install individually.
- **Balance Logs:**
  - **Stable Receiver:** `stablelog1.json`
  - **Stable Provider:** `stablelog2.json`
  - Located in `~/.lightning/bitcoin/stablechannels/`

For CLN, clone this repo, or create a `stablechannels.py` file with the contents of `stablechannels.py` for CLN. 

### Connecting and creating a dual-funded channel (for CLN)

If your Lightning Node is running, you will need to stop your Lightning Node and restart it with the proper commands for dual-funded (or interactive) channels.

You can do this with the following commands.

Stop Lightning: `lightning-cli stop` or `lightning-cli --testnet stop`.

Next, start your CLN node, or modify your config files, to enable dual-funding channels up to the amount you want to stabilize, or leverage. This will look like this

```bash
lightningd --daemon --log-file=/home/ubuntu/cln.log --lightning-dir=/home/ubuntu/lightning --experimental-dual-fund --funder-policy=match --funder-policy-mod=100 --funder-min-their-funding=200000 --funder-per-channel-max=300000 --funder-fuzz-percent=0 --lease-fee-base-sat=2sat --lease-fee-basis=50 --experimental-offers --funder-lease-requests-only=false
```
The "funder" flags instruct CLN on how to handle dual-funded channels. Basically this command is saying: "This node is willing to fund a dual-funded channel up to **300000** sats, a minimum of **200000** sats, plus some other things not relevant for Stable Channels.  

Your counterparty will need to run a similar command. 

Next connect to your counterparty running the CLN `connect` command. This will look something like: `lightning-cli connect 021051a25e9798698f9baad3e7c815da9d9cc98221a0f63385eb1339bfc637ca81 54.314.42.1`

Now you are ready to dual-fund. This command will look something like `lightning-cli fundchannel 021051a25e9798698f9baad3e7c815da9d9cc98221a0f63385eb1339bfc637ca81 0.0025btc`

If all goes well, we should be returned a txid for the dual-funded channel, and  both parties should have contributed 0.0025btc to the channel. 

Now this needs to be confirmed on the blockchain. 

### Starting Stable Channels

We need to start the Stable Channels plugin with the relevant details of the Stable Channel.

The plugin startup command will look something like this for CLN:

```bash
lightning-cli plugin subcommand=start plugin=/home/clightning/stablechannels.py channel-id=b37a51423e67a1f6733a78bb654535b2b81c427435600b0756bb65e21bdd411a stable-dollar-amount=95 is-stable-receiver=True counterparty=026b9c2a005b182ff5b2a7002a03d6ea9d005d18ed2eb3113852d679b3ec3832c2 native-btc-amount=0
```

Modify the directory for your plugin.

What this command says is: "Start the plugin at this directory. Make the Lightning channel with channel ID b37a51423e67a1f6733a78bb654535b2b81c427435600b0756bb65e21bdd411a a stable channel at $95.00. and 0 sats of BTC. Is is `True` that the node running this command is the Stable Receiver. Here's the ID of the counterparty `026b9c..`."

Your counterparty will need to run a similar command, and the Stable Channels software should do the rest. 

The startup command for the LND plugin will be something like this:

```bash
python3 lnd.py 
    --tls-cert-path=/Users/alice/tls.cert
    --expected-dollar-amount=100 
    --channel-id=137344322632000 
    --is-stable-receiver=false 
    --counterparty=020c66e37461e9f9802e80c16cc0d97151c6da361df450dbca276478dc7d0c271e 
    --macaroon-path=/Users/alice/admin.macaroon 
    --native-amount-sat=0 
    --lnd-server-url=https://127.0.0.1:8082
```

Stable Channel balance results for the Stable Receiver are written to the `stablelog1.json` file  and logs for the Stable Provider are written to the `stablelog2.json` file. 

##  Payout matrix

Assume that we enter into a stable agreement at a price of $60,000 per bitcoin. Each side puts in 1 bitcoin, for a total channel capacity of 2 bitcoin, and a starting USD nominal value of $120,000 total. The below table represents the payouts and percentage change if the bitcoin price increases or decreases by 10%, 20%, or 30%. Check out this payout matrix to better understand the mechanics of the trade agreement.

Abbreviations:
- SR = Stable Receiver
- SP = Stable Provider
- Δ = Delta / Change

| Price Change (%) | New BTC Price | SR (BTC) | SR (USD) | SP (BTC) | SP (USD) | SR Fiat Δ$ | SR BTC Δ | SR Fiat Δ% | SR BTC Δ% | SP Fiat Δ$ | SP BTC Δ | SP Fiat Δ% | SP BTC Δ% |
|------------------|---------------|----------|----------|----------|----------|------------|----------|------------|----------|------------|----------|------------|----------|
| -30              | 42000.0       | 1.4286   | 60000    | 0.5714   | 42000.0  | 0          | +0.4286  | 0%         | +42.86%  | -18000.0   | -0.4286  | -60%       | -42.86%  |
| -20              | 48000.0       | 1.25     | 60000    | 0.75     | 48000.0  | 0          | +0.25    | 0%         | +25%     | -12000.0   | -0.25    | -40%       | -25%     |
| -10              | 54000.0       | 1.1111   | 60000    | 0.8889   | 54000.0  | 0          | +0.1111  | 0%         | +11.11%  | -6000.0    | -0.1111  | -20%       | -11.11%  |
| 0                | 60000.0       | 1        | 60000    | 1        | 60000.0  | 0          | 0        | 0%         | 0%       | 0          | 0        | 0%         | 0%       |
| 10               | 66000.0       | 0.9091   | 60000    | 1.0909   | 66000.0  | 0          | -0.0909  | 0%         | -9.09%   | +6000.0    | +0.0909  | +20%       | +9.09%   |
| 20               | 72000.0       | 0.8333   | 60000    | 1.1667   | 72000.0  | 0          | -0.1667  | 0%         | -16.67%  | +12000.0   | +0.1667  | +40%       | +16.67%  |
| 30               | 78000.0       | 0.7692   | 60000    | 1.2308   | 78000.0  | 0          | -0.2308  | 0%         | -23.08%  | +18000.0   | +0.2308  | +60%       | +23.08%  |


## Roadmap

Hope to move all this to issues and PRs soon.

#### Done:
- [x] bash script version
- [x] first CLN plugin version
- [x] LND version
- [x] first Python app version
- [x] test Greenlight integration
- [x] price feed integration
- [x] UTXOracle plugin - https://github.com/toneloc/plugins/blob/master/utxoracle/utxoracle.py
- [x] dual-funded flow
- [x] mainnet deployment
- [x] Add native field / partially stable
- [x] user feedback on CLN plugin
- [x] LDK version

#### To do:
- [ ] LSP just-in-time channel integration
- [ ] read-only iPhone app published in App Store
- [ ] manage channel creation via `fundchannel` command
- [ ] monitor channel creation tx, and commence `check_stables` after
- [ ] move Stable Channels details to conf files (*)
- [ ] use CLN `datastore` command to manage Stable Channel details (?)
- [ ] accounting commands
- [ ] Python Greenlight integration
- [ ] trading web app
- [ ] VLS integration
- [ ] read-only Android app published in App Store
- [ ] crypto keys on mobile
- [ ] FinalBoss plugin

## Rationale and Challenges

This Delving Bitcoin post goes more in-depth on challenges and opportunities - https://delvingbitcoin.org/t/stable-channels-peer-to-peer-dollar-balances-on-lightning

### Acknowledgements

Thanks to Christian Decker and the Core Lightning team from Blockstream for his help with setting up Greenlight. Thanks to Michael Schmoock (m-schmoock) for writing the "currencyrate" plugin, which I use. Thanks to @jamaljsr for developing the Polar Lightning Network visualization tool. I also used Jamal's code for the Stable Channels.com website. Thanks to Dan Robinson for his work on Rainbow Channels. Thanks to Daywalker90 and StarBuilder for open-source contributions.

Thanks to all of the Lightning Network core developers, and all of the bitcoin open-source devs on whose giant shoulders we stand. 
