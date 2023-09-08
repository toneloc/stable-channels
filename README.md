## Peer Stables

Peer Stables provides two services to users:
<ol>
  <li>synthetic USD exposure to <i><stableReceivers</i> .... this is effectively a 1x shorrt on BTC/USD</li>
  <li>1x leveraged long exposure to <i>stableReceivers</i></li>
</ol>
synthetic USD 
Top use-cases

stableReceiver // stableProvider

stable USD balance // leveraged long
bitcoin-native, self-custodial yield -> yield on routing Lightning payments (maintaing multiple channels?) -> yield on degens going leveraged long (must build funding rate oracle) -> CL Boss for stable channels **
add leverage x3
instant. settlement -> assets and derivatives ...
synth Apple stock
synth currency
synth options
How to match counterparties - marketing-making

web app -> money-making
trigger US regs. on stablecoins, derivativies trading ... ?
easier to get started ... "im looking for $100 stable" ...
more p2p -> not money making
Liquidity ads from Blockstream where you advertise that you want to open a channel over the LN
modify Liquidity ads
big work to do: - handling the oracles - offline? index price? binance .... - gives some outrageous price ... handle that? - marketing-making? - renogtiation of stable agreements - payments - handling all of the complicaitons of the LN - whats the final factor? - retail - app? web app - insts. - web app dashboard? software they run themselves - core lightning blockstream -> plugins **** - wallet providers ... backend system for them to help customers get yield - crypto exchanges

benefits - p2p stables ... different concept - exists on Eth, not on LN / bitcoin - non-custodial (does tether have the money?) - possibly: cant get shut down by US gov - fail fast mentality - self-custodial yield

problems - inherits all problems of LN - money is in a hot wallet - always online .. - uses oracles - there are still ways for you to get f*cked, but we try to fail fast - lot of code to be written - not efficient

consistent bull market attack ... close once youve made a lot of money ...

make a actual service -> Java or, do a cmd line SQL entry and have the JS web app read from DB - find an open source wallet and modify it

sidecar channels -> lightning pool - lightning labs

Sample Python Connections:

`` python3 peerStables.py f561cb7d56b0d033172490ee9c281d9e4d0b6f44ff0cccb07a6c6a5091919779 AgELYy1saWdodG5pbmcCPlNhdCBOb3YgMDUgMjAyMiAwNTowMzoxMiBHTVQrMDAwMCAoQ29vcmRpbmF0ZWQgVW5pdmVyc2FsIFRpbWUpAAAGIBcnf+0eDYq75V0fKEN42ulqrTHPRQAJ0JY6MBTaLAV3 True 50 http://127.0.0.1:8183

curl -s -H "macaroon:02010b632d6c696768746e696e6702374672692041756720313920323032322030333a34313a303820474d542b303130302028427269746973682053756d6d65722054696d652900000620102c414192d83c9a7031501ff43bf2a145c150056c2ac7f9d083906abfecbb04" http://192.168.1.221:6100/v1/getinfo | jq -r '.id')

``

Peer Stable state maps to Core Lightning states. These state mappings will change with dual-funded channels.

"OPENINGD": The channel funding protocol with the peer is ongoing and both sides are negotiating parameters.    "CHANNELD_AWAITING_LOCKIN": The peer and you have agreed on channel parameters and are just waiting for the channel funding transaction to be confirmed deeply. Both you and the peer must acknowledge the channel funding transaction to be confirmed deeply before entering the next state. "CHANNELD_NORMAL": The channel can be used for normal payments. "CHANNELD_SHUTTING_DOWN": A mutual close was requested (by you or peer) and both of you are waiting for HTLCs in-flight to be either failed or succeeded. The channel can no longer be used for normal payments and forwarding. Mutual close will proceed only once all HTLCs in the channel have either been fulfilled or failed. "CLOSINGD_SIGEXCHANGE": You and the peer are negotiating the mutual close onchain fee. "CLOSINGD_COMPLETE": You and the peer have agreed on the mutual close onchain fee and are awaiting the mutual close getting confirmed deeply. "AWAITING_UNILATERAL": You initiated a unilateral close, and are now waiting for the peer-selected unilateral close timeout to complete. "FUNDING_SPEND_SEEN": You saw the funding transaction getting spent (usually the peer initiated a unilateral close) and will now determine what exactly happened (i.e. if it was a theft attempt). "ONCHAIN": You saw the funding transaction getting spent and now know what happened (i.e. if it was a proper unilateral close by the peer, or a theft attempt). "CLOSED": The channel closure has been confirmed deeply. The channel will eventually be removed from this array.
