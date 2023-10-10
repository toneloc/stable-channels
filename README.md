## Stable Channels

<i>View available Stable Channel offers on https://www.stablechannels.com.</i>

<b>Stable Channels</b> let Lightning Network node runners keep one of their channel balances stable in dollar terms, for example $100. These special channels are called <b>Stable Channels</b>. These node runners are called <b>Stable Receivers</b>.

On the other side of the channel are <b>Stable Providers</b>. Stable Providers go 1x leveraged long bitcoin. This is what that means:
<ul>
<li>If the price of bitcoin goes up, the Stable Provider gets more bitcoin. This is because it takes less bitcoin to keep the Stable Receiver stable in dollar terms.
<li>If the price of bitcoin goes down, the Stable Provider loses bitcoin. This is because it takes more bitcoin to keep the Stable Receiver stable in dollar terms.
</ul>

These two nodes query price feeds at regular intervals. Then, based on the new price, they update their channel balance to keep the Stable Receiver stable at $100 of bitcoin. Either party may opt out at anytime, either by a cooperative or forced channel close. 

Stable Channels are unannounced to the public network and are non-routing channels. These are vanilla Lightning channels with no DLCs.

Stable Channels work on "Core Lightning" -- which is Blockstream's implementation of the Lightning Network <link>https://www.github.com/BOLTs <link>. 

A Stable Channels work like this:

<ol>
<li>Match with a counterparty and come to an agreement on the parameters of the Stable Channel. 
<li>Select the price feeds. By default, Stable Channels uses the median of five price feeds: BitStamp, Coinbase, CoinGecko, Coinbase, and BitBlock.
<li>Create a dual-funded channel with the counterparty, each putting in the full amount of the Stable Channel. 
<ul>
<li> <i>Example: If the Stable Channel is for $100, each side of the channel puts in $100, for a total channel capacity of $200</i>
</ul>
<li>Query the price feeds' APIs and update the Stable Channel balance accordingly:
<ul>
<li>If the price went down, the Stable Provider needs to pay the Stable Receiver. 
<li>If the price went up, the Stable Receiver needs to pay the Stable Provider.
<li>If the price stayed the same or moved only a tiny amount, no payment is needed.
</ul>
</ol>

Example:

Let's break this down:

Stable Receiver: This actor desires stability in terms of USD. They do not want to benefit or lose from Bitcoin price fluctuations.
Stable Provider: This actor is willing to absorb the Bitcoin price fluctuation risk in exchange for potential profits.
Scenario:

Starting Point: Both users put in $100 of Bitcoin into the pot, so the pot holds $200 worth of Bitcoin.
Now, consider three potential outcomes:

A. Bitcoin price remains the same.
B. Bitcoin price increases by 10%.
C. Bitcoin price decreases by 10%.

A. Price remains the same:

Stable Receiver: Gets back their $100.
Stable Provider: Gets back their $100.
B. Price increases by 10%:
The $200 worth of Bitcoin in the pot becomes $220.

Stable Receiver: Still gets back only their $100. They forfeit the additional $10 that their original Bitcoin would have earned.
Stable Provider: Gets their original $100 plus the $10 forfeited by the Stable Receiver. So, they receive $110.
C. Price decreases by 10%:
The $200 worth of Bitcoin in the pot becomes $180.

Stable Receiver: Gets back their original $100. This means they get an extra $10 from the pot to offset the Bitcoin price drop.
Stable Provider: Gets only $90 because they had to cover the $10 to keep the Stable Receiver whole.
Which side is leveraged short/long?

Stable Receiver: Is effectively leveraged short on Bitcoin. This is because they profit when Bitcoin goes down (compared to not being in this agreement). The exact leverage depends on the total change in price, but in our scenario where Bitcoin moves by 10%, the leverage is essentially 2x, because a 10% drop results in a 20% gain for the Stable Receiver.

Stable Provider: Is effectively leveraged long on Bitcoin. They benefit when Bitcoin rises but take the hit when it falls. Again, using our 10% price change scenario, they are 2x leveraged long on Bitcoin because a 10% increase gives them a 20% gain.

In essence, the Stable Provider is doubling down on the volatility of Bitcoin, hoping for upward movement, while the Stable Receiver is seeking stability and protection against downward movement.

## Getting started

Terminal access to a "Core Lighting" node is required.

Access or create the `/plugins` folder on your node. and `cd` into this folder.

Run `git clone stablechannels.com`

This will create the `stable-channels` folder in `/plugins`.

If your Lightning Node is running, you will need to restart your Lightning Node to run the plugin. You can do this with the following commands. These commands are written for testnet.

Stop Lightning: `lightning-cli --testnet stop`

Restart Lightning with the plugin: `lightningd --daemon --network=testnet --plugin=/home/ubuntu/plugins/`

If you already have a channel and it is correctly balanced, then change if to Stable Channel with the following flags:

<ul>
<li>1st parameter = short channel ID -> <i>Example:</i> `2440124x15x0`
<li>2nd parameter = amount, in dollars, that you want to hold stabl, for example 100
<li>3rd parameter = whether you are the stable receiver or not. Put `True` if you are the `Stable Receiver` and `False` if you are the `Stable Provider`
</ul>

The full command might look like this: `stable-channels 2440124x15x0 100 0.2 True`

## Workflow Example

Make this a markdown table ... 

Imagine the following stable agreement for $25,000.

time / expected dollar amount / stableReceiver balance / stable provider balance / btc price / expected action from prior row
0 / / 25001 / 1 / 25000
1 / 2 /
2 / 3 /

## Rationale and Challenges

The most valuable stablecoins today are Tether and USDX. These stablecoins hold their value in cash and bonds. This cash and these bonds have custodians. These custodians are centralized companies and may be forced to freeze these assets or revoke banking access. Either of these may mark the effective failure of that stablecoin to retain its purchasing power. 

Stable Channels intends to provide a more socially scalable solution. Stable Channels, as a solution, is self-custodial, has no token or token issuer, and intends to give a real-time, streaming finance experience for its users. 

Stable Channels faces challenges. Stable Channels inherit many of the challenges of the Lightning Network. One challeges is that with Lightning, bitcoin is held in an online wallet. Another challenges is that both nodes must always be online. Yet another challenge is getting trustworthy price feeds. Finally, there are various cyber and social engineering attacks that we can envision. 

The Stable Channels approach is that while all of these failure modes and attacks are plausible, it is only by building a USD experience on top of <i>only bitcoin</i> that we can give users the best USD-like experience. 

For those users who want USD experience and are Americans, we recommmend FDIC-insured bank accounts. 

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

## Splicing workflow

Stable Channels intends to use channel splicing to handle "margin calls."

Splicing works as follows:



## Greenlight 

Stable Channels intends to integrate with Blockstream's Greenlight product. 


## Acknowledgements

Thanks to Christian Decker and the Core Lightning team from Blockstream for his help with setting up Greenlight. Thanks to Michael Schmoock (m-schmoock) for writing the "currencyrate" plugin, which I use. Thanks to @jamaljsr for developing the Polar Lightning Network visualization tool. I also used Jamal's code for the Stable Channels.com website. Thanks to Dan Robinson for his work on Rainbow Channels.

Thanks to all of the Lightning Network core developers, and all of the Bitcoin open-source devs on whose giant shoulders we stand. 
