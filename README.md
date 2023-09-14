## PeerStables

PeerStables lets Lightning Network node runners keep one of their channel balances stable in dollar terms. These channels are called <i>stable channels</i>. These node runners are called stable receivers.

Alternatively, PeerStables lets node runners go 1x leveraged long bitcoin on the other side of the channel. These node runners are called stable providers.

These nodes query price oracles and update the channel balance accordingly. Either party may opt out at anytime, either by a cooperative or forced close. 

Stable channels are currently unannounced to the public network and are non-routing channels.

Stable channels currently works on "Core Lightning," Blockstream's implmenetation of the Lightning Network. 

It works like this:

<ol>
<li>Match with a counterparty and come to an agreement on the parameters of the stable channel agreement. 
<li>Select the price oracles. By default, PeerStables uses the median of five price oracles: BitStamp, Coinbase, CoinGecko, Coinbase, and BitBlock.
<li>Create a dual-funded channel with the counterparty, each putting in the amount of the stable channel. 
<ul>
<li> <i>Example: If the stable channel is for $100, each side of the channel puts in $100, for a total channel capacity of $200</i>
</ul>

<li>Query the price oracle and update the stable channel balance accordingly:
<ul>
<li>If the price went down, the Stable Provider needs to pay the Stable Receiver. 
<li>If the price went up, the Stable Receiver needs to pay the Stable Provider.
<li>If the price stayed the same or moved only a tiny amount, no paymeny is needed.

</ul>
</ol>

Note: Find example workflows in the "Example Workflows" section.
</ol>

Create a channel with the following flags

```
# Stable Channel flags
--short-channel-ID=2440124x15x0
--stable-amount=100
--minimum-margin-ratio=0.2
--is-stable-receiver=True
--channel-id=030d21990f4c6394165aabd43e793ea572b822fa33c2fd2c7f9b406315e191234c
--rpc-path=/home/ubuntu/.lightning/lightning-rpc
```

## Workflow Examples

Make this a markdown table ... 

Imagine the following stable agreement for $25,000.

time / expected dollar amount / stableReceiver balance / stable provider balance / btc price / action from prior row
0 / / 25001 / 1 / 25000
1 / 2 /
2 / 3 /

## Rationale and Challenges

The value behind most stablecoins today is custodial. USDC and Tether face regulatroy risk of having their assets freezed and banking access revoked. 

PeerStables intends to provide a more soscially scalable solution. PeerStables is self-custodial, has no token or token issuer, and have a real-time streaming finance experience for bitcoin Lightning network. 

PeerStables faces some challenges. For example, Peer Stables inherits many of the challenges of the Lightning Network such as that bitcoin is held in a online wallet and both nodes must always be online.

Furthermore, there are various attacks we can envision. The PeerStables approach is that while all of these attacks are plausible, it is only by building a USD experience on top of <i>only bitcoin</i> that we can, paradoxically, give users without access to the US banking system a stable USD asset. 

## Interactive channel open workflow

Peer Stable intends use interactive channel opening  to neogtiate the terms of the stable agreement and start a well-balanced channel.  

The Channel state in CLN map to Core Lightning states to Peer Stable states, but have several differences. 


| CLN State               | Description                                                                                                                                                                       | Peer Stables differences |
|-------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------|
| OPENINGD                | The channel funding protocol with the peer is ongoing, and both sides are negotiating parameters.                                                                               |                          |
| CHANNELD_AWAITING_LOCKIN| The peer and you have agreed on channel parameters and are just waiting for the channel funding transaction to be confirmed deeply. Both you and the peer must acknowledge the channel funding transaction to be confirmed deeply before entering the next state. |                          |
| CHANNELD_NORMAL         | The channel can be used for normal payments.                                                                                                                                     |                          |
| CHANNELD_SHUTTING_DOWN  | A mutual close was requested (by you or peer), and both of you are waiting for HTLCs in-flight to be either failed or succeeded. The channel can no longer be used for normal payments and forwarding. Mutual close will proceed only once all HTLCs in the channel have either been fulfilled or failed. |                          |
| CLOSINGD_SIGEXCHANGE    | You and the peer are negotiating the mutual close onchain fee.                                                                                                                    |                          |
| CLOSINGD_COMPLETE       | You and the peer have agreed on the mutual close onchain fee and are awaiting the mutual close getting confirmed deeply.                                                          |                          |
| AWAITING_UNILATERAL     | You initiated a unilateral close, and are now waiting for the peer-selected unilateral close timeout to complete.                                                                |                          |
| FUNDING_SPEND_SEEN      | You saw the funding transaction getting spent (usually the peer initiated a unilateral close) and will now determine what exactly happened (i.e. if it was a theft attempt).    |                          |
| ONCHAIN                 | You saw the funding transaction getting spent and now know what happened (i.e. if it was a proper unilateral close by the peer, or a theft attempt).                            |                          |
| CLOSED                  | The channel closure has been confirmed deeply. The channel will eventually be removed from this array.                                                                         |                          |

## Splicing workflow

PeerStables intends to use channel splicing to handle "margin calls" ... 

## Greenlight 



## Acknowledgements

Thanks to Christian Decker and the Core Lightning from the Blockstream team for his help with setting up Greenlight. Thanks to Dan Robinson for weriting about Rainbow Channels a number of year ago. Thanks to Michael Schmoock (m-schmoock) for writing the "currencyrate" plugin, which I use. Thanks to @jamaljsr for developing the Polar Lightning Network visualization tool. 

Thanks to all of the Lightning Network core developers, and all of the Bitcoin open-source developers on whose giant shoulders we stand. 