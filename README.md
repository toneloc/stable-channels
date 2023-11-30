
#### Note: Skip to [Getting Started](#getting-started) for technical startup instructions.

## Stable Channels

This Twitter thread explains things pretty well, with an example - https://x.com/tonklaus/status/1729567459579945017

<b>Stable Channels</b> lets Lightning Network node runners keep one of their channel balances stable in dollar terms, for example $100. These special channels are called <b>Stable Channels</b>. These node runners are called <b>Stable Receivers</b>.

On the other side of the channel are <b>Stable Providers</b>. Stable Providers want to leverage their bitcoin. In simple terms, this means that Stable Providers want to use their Bitcoin to get more Bitcoin. However, Stable Receivers put their Bitcoin at risk by doing so.

These two nodes query price feeds every 5 minutes. Then, based on the new price, they update their channel balance with their counterparty to keep the Stable Receiver stable at $100 of bitcoin. Each party remains self-custodial. Either party may opt out at anytime, either by a cooperative on-chain channel close or forced channel close. 

This basic process works as follows:
<ul>
<li>If the price of bitcoin goes up, the Stable Provider gets more bitcoin. This is because it takes less Bitcoin to keep the Stable Receiver stable in dollar terms, so the Stable Receiver pays the Stable Provider. In the base case, the Stable Provider has a 2x long bitcoin position. 
<li>If the price of bitcoin goes down, the Stable Provider loses bitcoin. This is because it takes more Bitcoin to keep the Stable Receiver stable in dollar terms, so the Stable Provider pays the Stable Receiver.
</ul>

Stable Channels are unannounced to the public network and are non-routing channels. Technologially, these are vanilla Lightning channels with no DLCs, and there are no tokens or fiat on-ramps involved.

Stable Channels work on "Core Lightning," which is Blockstream's implementation of the Lightning Network specification. 

A Stable Channels work like this:

<ol>
<li>Match with a counterparty and come to an agreement on the parameters of the Stable Channel. 
<li>Select the price feeds. By default, Stable Channels takes the median of five price feeds: BitStamp, Coinbase, CoinGecko, Coinbase, and BitBlock.
<li>Create a dual-funded channel with the counterparty, each putting in the amount of the Stable Channel. 
<ul>
<li> <i>Example: If the Stable Channel is for $100, each side of the channel puts in $100, for a total channel capacity of $200 at the time of channel creation</i>
</ul>
<li>Query the five price feeds' APIs and update the Stable Channel balance accordingly:
<ul>
<li>If the price went down, the Stable Provider needs to pay the Stable Receiver. 
<li>If the price went up, the Stable Receiver needs to pay the Stable Provider.
<li>If the price stayed the same or moved only a tiny amount, no payment is required
</ul>
</ol>

## Getting Started

### Enivronment and dependencies

- Terminal access to bitcoind and a CLN node running version `23.05.2` is required. Other versions may work but `23.08.1` does not work.
- Python3 is also required. 

Clone this repo, or create a `stablechannels.py` file with the contents of `stablechannels.py`. 

Stable Channels has a few dependencies. 
- Either copy the `requirements.txt` file and run `pip3 install -r requirements.txt`.
- Or: `python3 install` each of the five dependencies listed in `requirements.txt`.

### Connecting and creating a dual-funded channel

If your Lightning Node is running, you will need to stop your Lightning Node and restart it with the proper commands for dual-funded (or interactive) channels.

You can do this with the following commands.

Stop Lightning: `lightning-cli stop` or `lightning-cli --testnet stop`.

Next, start your CLN node, or modify your config files, to enable dual-funding channels up to the amount you want to stablize, or leverage. This will look like this

```bash
lightningd --daemon --log-file=/home/ubuntu/cln.log --lightning-dir=/home/ubuntu/lightning --experimental-dual-fund --funder-policy=match --funder-policy-mod=100 --funder-min-their-funding=200000 --funder-per-channel-max=300000 --funder-fuzz-percent=0 --lease-fee-base-sat=2sat --lease-fee-basis=50 --experimental-offers --funder-lease-requests-only=false
```
THe "funder" flags instruct CLN on how to handle dual-funded channels. Bascially this command is saying: "This node is willing to fund a dual-funded up to **300000** sats, a minimum of **200000** sats, plus some other things not relevant for Stable Channels.  

Your counterparty will need to run a similary command. 

Next connect to your counterparty running the CLN `connect` command. This will look something like: `lightning-cli connect 021051a25e9798698f9baad3e7c815da9d9cc98221a0f63385eb1339bfc637ca81 54.314.42.1`

Now you are ready to dual-fund. This command will look something like `lightning-cli fundchannel 021051a25e9798698f9baad3e7c815da9d9cc98221a0f63385eb1339bfc637ca81 0.0025btc`

If all goes well, we should be returned a txid for the dual-funded channel, and  both parties should have contributed 0.0025btc to the channel. 

Now this needs to be confirmed on the blockchain. 

### Starting Stable Channels

First let's create the log file. If you are the stable receiver, your logs get writtent to `stablelog1.json`. Create that file

We need to restart Lightning running the plugin and with the relevant details of the Stable Channel.

Stop Lightning: `lightning-cli stop` or `lightning-cli --testnet stop`.

The startup command will look something like this:

```bash
lightningd --daemon --log-file=/home/ubuntu/cln.log --experimental-dual-fund --funder-policy=match --funder-policy-mod=100 --funder-min-their-funding=1000 --funder-per-channel-max=300000 --funder-fuzz-percent=0 --lease-fee-base-sat=2sat --lease-fee-basis=50 --experimental-offers --funder-lease-requests-only=false --plugin=/home/ubuntu/stablechannels.py --stable-details=515501x1272x1,100,0.2,True,021051a25e9798698f9baad3e7c815da9d9cc98221a0f63385eb1339bfc637ca81,/home/ubuntu/.lightning/bitcoin/lightning-rpc
```

What this command says is: "Make the Lightning channel with short ID a stable channel at $100.00. Require the Stable Provider counterparty maintain 20% of the par value of the peg amount on his side of the channel. Is is `True` that the node running this commmand is the Stable Receiver. Here's the ID of the counterparty `02105..` and here's the RPC path."

Your counterparty will need to run a similar command, and the Stable Channels software should do the rest. 

Logs for the Stable Receiver a are written to `stablelog1.json` file  and logs for the Stable Provider are written to the `stablelog2.json` file. 



##  Payout matrix

Assume that we enter into a stable agreement at a price of $27,500 per Bitcoin. Each side puts in 1 Bitcoin, for a total channel capacity of 2 Bitcoin, and a starting USD nominal value of $55,000 total. The below table represents the payouts and percentage change if the bitcoin price increases or decreases by 10%, 20%, or 30%. Check out this payout matrix to better understand the mechanics of the trade agreement.

Abbreviations:
- SR = Stable Receiver
- SP = Stable Provider
- Δ = Represents change

| Price Change (%) | New BTC Price | SR (BTC) | SR (USD) | SP (BTC) | SP (USD) | SR Fiat Δ$ | SR BTC Δ | SR Fiat Δ% | SR BTC Δ% | SP Fiat Δ$ | SP BTC Δ | SP Fiat Δ% | SP BTC Δ% |
|------------------|---------------|----------|----------|----------|----------|------------|----------|------------|----------|------------|----------|------------|----------|
| -30              | $19,250      | 1.4286   | $27,500  | 0.5714   | $11,000  | $0         | +0.4286  | 0%         | +42.86%  | -$16,500   | -0.4286  | -60%       | -42.86%  |
| -20              | $22,000      | 1.25     | $27,500  | 0.75     | $16,500  | $0         | +0.25    | 0%         | +25%     | -$11,000   | -0.25    | -40%       | -25%     |
| -10              | $24,750      | 1.1111   | $27,500  | 0.8889   | $22,000  | $0         | +0.1111  | 0%         | +11.11%  | -$5,500    | -0.1111  | -20%       | -11.11%  |
| 0                | $27,500      | 1        | $27,500  | 1        | $27,500  | $0         | 0       | 0%         | 0%       | $0         | 0       | 0%         | 0%       |
| 10               | $30,250      | 0.9091   | $27,500  | 1.0909   | $33,000  | $0         | -0.0909 | 0%         | -9.09%   | +$5,500    | +0.0909 | +20%       | +9.09%   |
| 20               | $33,000      | 0.8333   | $27,500  | 1.1667   | $38,500  | $0         | -0.1667 | 0%         | -16.67%  | +$11,000   | +0.1667 | +40%       | +16.67%  |
| 30               | $35,750      | 0.7692   | $27,500  | 1.2308   | $44,000  | $0         | -0.2308 | 0%         | -23.08%  | +$16,500   | +0.2308 | +60%       | +23.08%  |


## Roadmap

Hope to move all this to issues and PRs soon.

#### Done:
- [x] bash script version
- [x] first CLN plugin version
- [x] first Python version
- [x] CLI Greenlight integration
- [x] price feed integration
- [x] UTXOracle plugin - https://github.com/toneloc/plugins/blob/master/utxoracle/utxoracle.py
- [x] dual-funded flow
- [x] mainnet deployment

#### To do:
- [ ] manage channel creation via `fundchannel` command
- [ ] monitor channel creation tx, and commence `check_stables` after
- [ ] move Stable Channels details to conf files (*)
- [ ] user feedback on CLN plugin
- [ ] use CLN `datastore` command to manage Stable Channel details (?)
- [ ] accounting commands
- [ ] Python Greenlight integration
- [ ] trading web app
- [ ] VLS integration
- [ ] mobile <-> RPC Greenlight integration
- [ ] read-only iPhone app published in App Store
- [ ] read-only Android app published in App Store
- [ ] crypto keys on mobile
- [ ] FinalBoss plugin

## Rationale and Challenges

The most valuable stablecoins today are Tether and USDC. These stablecoins hold their value in fiat: cash and bonds. This cash and these bonds have fiat custodians. These fiat custodians are centralized companies that may be forced to freeze these assets or revoke banking access. Either of these scenarios mark a liveness failure of that stablecoin to retain its purchasing power, or worse.

Stable Channels intends to provide a more socially scalable solution. Stable Channels, as a solution, is self-custodial, has no token or token issuer, and intends to give a real-time, streaming finance experience for its users. The vision is to create a self-custoodial derivatives exchange where users can hedge or lever up their Bitcoin exposure.

Stable Channels inherit many of the challenges of the Lightning Network. One challeges is that with Lightning, Bitcoin is held in an online wallet. Another challenges is that both nodes must always be online. Yet another challenge is getting trustworthy price feeds. Finally, there are various potential cyber and social engineering attacks.

The Stable Channels approach is that while all of these failure modes and attacks are plausible, it is only by building a USD experience on top of <i>only bitcoin</i> that we can, over the longterm, give users the best derivatives trading experience. 

For those users who want USD experience and are Americans, we recommmend FDIC-insured bank accounts. For those users who want Bitcoin exposure, we recommend simply HODLing spot Bitcoin.

## Interactive channel open workflow

Stable Channels intends to use interactive channel opening to neogtiate the terms of the stable agreement and start a well-balanced channel.  

The Channel state in CLN map to Core Lightning states to Stable Channel states, but have several differences. 


| Core Lightning State               | Description                                                                                                                                                                       | Stable Channel differences |
|------------|------------|------------|
| OPENINGD      | The channel funding protocol with the peer is ongoing, and both sides are negotiating parameters.                                                                               |              Instead of using the ``fund-channel`` command, use the ``create-stable-channel command`` with the parameters listed above in the "Getting Started" section.          |
| CHANNELD_AWAITING_LOCKIN| The peer and you have agreed on channel parameters and are just waiting for the channel funding transaction to be confirmed deeply. Both you and the peer must acknowledge the channel funding transaction to be confirmed deeply before entering the next state. |        No difference.                  |
| CHANNELD_NORMAL         | The channel can be used for normal payments.                                                                                                                                     |           No difference. However, the Stable Channels plugin is monitoring the behavior of the counterparty.               |
| CHANNELD_SHUTTING_DOWN  | A mutual close was requested (by you or peer), and both of you are waiting for HTLCs in-flight to be either failed or succeeded. The channel can no longer be used for normal payments and forwarding. Mutual close will proceed only once all HTLCs in the channel have either been fulfilled or failed. |                 No difference.         |
| CLOSINGD_SIGEXCHANGE    | You and the peer are negotiating the mutual close onchain fee.                                                                                                                    | No difference.                         |
| CLOSINGD_COMPLETE       | You and the peer have agreed on the mutual close onchain fee and are awaiting the mutual close getting confirmed deeply.                                                          | No difference.                          |
| AWAITING_UNILATERAL     | You initiated a unilateral close, and are now waiting for the peer-selected unilateral close timeout to complete.                                                                |            No difference.              |
| FUNDING_SPEND_SEEN      | You saw the funding transaction getting spent (usually the peer initiated a unilateral close) and will now determine what exactly happened (i.e. if it was a theft attempt).    |                 No difference.         |
| ONCHAIN                 | You saw the funding transaction getting spent and now know what happened (i.e. if it was a proper unilateral close by the peer, or a theft attempt).                            |         No difference.                 |
| CLOSED                  | The channel closure has been confirmed deeply. The channel will eventually be removed from this array.                                                                         |           No difference.               |

### Splicing workflow

Stable Channels intends to use channel splicing to handle "margin calls."


### Greenlight 

Stable Channels intends to integrate with Blockstream's Greenlight product. 


### Acknowledgements

Thanks to Christian Decker and the Core Lightning team from Blockstream for his help with setting up Greenlight. Thanks to Michael Schmoock (m-schmoock) for writing the "currencyrate" plugin, which I use. Thanks to @jamaljsr for developing the Polar Lightning Network visualization tool. I also used Jamal's code for the Stable Channels.com website. Thanks to Dan Robinson for his work on Rainbow Channels.

Thanks to all of the Lightning Network core developers, and all of the Bitcoin open-source devs on whose giant shoulders we stand. 
