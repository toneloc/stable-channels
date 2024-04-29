#!/usr/bin/python3

# Stable Channels: p2p BTCUSD trading on Lightning
# Contents
# Section 1 - Dependencies and main data structure
# Section 2 - Price feed config and logic
# Section 3 - Core logic 
# Section 4 - Plug-in initialization

# Section 1 - Dependencies and main data structure
from pyln.client import Plugin # Library for CLN Python plug-ins created by Blockstream 
from pyln.client import Millisatoshi # Library for CLN Python plug-ins created by Blockstream 
from pyln.client import LightningRpc
from collections import namedtuple # Standard on Python 3
from cachetools import cached, TTLCache # Used to handle price feed calls; probably can remove
import requests # Standard on Python 3.7+
from requests.adapters import HTTPAdapter 
from requests.packages.urllib3.util.retry import Retry
import statistics # Standard on Python 3
import time # Standard on Python 3
from datetime import datetime 
from apscheduler.schedulers.blocking import BlockingScheduler # Used to check balances every 5 minutes
import threading # Standard on Python 3

plugin = Plugin()

class StableChannel:
    def __init__(
        self,
        plugin: Plugin,
        short_channel_id: str,
        expected_dollar_amount: float,
        native_amount_msat: int,
        is_stable_receiver: bool,
        counterparty: str,
        lightning_rpc_path: str,
        our_balance: float,
        their_balance: float,
        risk_score: int,
        stable_receiver_dollar_amount: float,
        stable_provider_dollar_amount: float,
        timestamp: int,
        formatted_datetime: str,
        payment_made: bool
    ):
        self.plugin = plugin
        self.short_channel_id = short_channel_id
        self.expected_dollar_amount = expected_dollar_amount
        self.native_amount_msat = native_amount_msat
        self.is_stable_receiver = is_stable_receiver
        self.counterparty = counterparty
        self.lightning_rpc_path = lightning_rpc_path
        self.our_balance = our_balance
        self.their_balance = their_balance
        self.risk_score = risk_score
        self.stable_receiver_dollar_amount = stable_receiver_dollar_amount
        self.stable_provider_dollar_amount = stable_provider_dollar_amount
        self.timestamp = timestamp
        self.formatted_datetime = datetime
        self.payment_made = payment_made

    def __str__(self):
        return (
            f"StableChannel(\n"
            f"    short_channel_id={self.short_channel_id},\n"
            f"    expected_dollar_amount={self.expected_dollar_amount},\n"
            f"    native_amount_msat={self.native_amount_msat},\n"
            f"    is_stable_receiver={self.is_stable_receiver},\n"
            f"    counterparty={self.counterparty},\n"
            f"    our_balance={self.our_balance},\n"
            f"    their_balance={self.their_balance},\n"
            f"    risk_score={self.risk_score},\n"
            f"    stable_receiver_dollar_amount={self.stable_receiver_dollar_amount},\n"
            f"    stable_provider_dollar_amount={self.stable_provider_dollar_amount},\n"
            f"    timestamp={self.timestamp},\n"
            f"    formatted_datetime={self.formatted_datetime},\n"
            f"    payment_made={self.payment_made}\n"
            f")"
        )
    
    @plugin.subscribe("coin_movement")
    def notify_coin_movement(plugin, coin_movement, **kwargs):
        l1 = LightningRpc(self.lightning_rpc_path)
        plugin.log("coin movement: {}".format(coin_movement))


        print(l1.listpays("null", coin_movement["payment_hash"]))


# Section 2 - Price feed config and logic
Source = namedtuple('Source', ['name', 'urlformat', 'replymembers'])

# 5 price feed sources
sources = [
    # e.g. {"high": "18502.56", "last": "17970.41", "timestamp": "1607650787", "bid": "17961.87", "vwap": "18223.42", "volume": "7055.63066541", "low": "17815.92", "ask": "17970.41", "open": "18250.30"}
    Source('bitstamp',
           'https://www.bitstamp.net/api/v2/ticker/btc{currency_lc}/',
           ['last']),
    # e.g. {"bitcoin":{"usd":17885.84}}
    Source('coingecko',
           'https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies={currency_lc}',
           ['bitcoin', '{currency_lc}']),
    # e.g. {"time":{"updated":"Dec 16, 2020 00:58:00 UTC","updatedISO":"2020-12-16T00:58:00+00:00","updateduk":"Dec 16, 2020 at 00:58 GMT"},"disclaimer":"This data was produced from the CoinDesk Bitcoin Price Index (USD). Non-USD currency data converted using hourly conversion rate from openexchangerates.org","bpi":{"USD":{"code":"USD","rate":"19,395.1400","description":"United States Dollar","rate_float":19395.14},"AUD":{"code":"AUD","rate":"25,663.5329","description":"Australian Dollar","rate_float":25663.5329}}}
    Source('coindesk',
           'https://api.coindesk.com/v1/bpi/currentprice/{currency}.json',
           ['bpi', '{currency}', 'rate_float']),
    # e.g. {"data":{"base":"BTC","currency":"USD","amount":"19414.63"}}
    Source('coinbase',
           'https://api.coinbase.com/v2/prices/spot?currency={currency}',
           ['data', 'amount']),
    # e.g. {  "USD" : {"15m" : 6650.3, "last" : 6650.3, "buy" : 6650.3, "sell" : 6650.3, "symbol" : "$"},  "AUD" : {"15m" : 10857.19, "last" : 10857.19, "buy" : 10857.19, "sell" : 10857.19, "symbol" : "$"},...
    Source('blockchain.info',
           'https://blockchain.info/ticker',
           ['{currency}', 'last']),
]

# Request logic is from "currencyrate" plugin: 
# https://github.com/lightningd/plugins/blob/master/currencyrate
def requests_retry_session(
    retries=3,
    backoff_factor=0.3,
    status_forcelist=(500, 502, 504),
    session=None,
):
    session = session or requests.Session()
    retry = Retry(
        total=retries,
        read=retries,
        connect=retries,
        backoff_factor=backoff_factor,
        status_forcelist=status_forcelist,
    )
    adapter = HTTPAdapter(max_retries=retry)
    session.mount('http://', adapter)
    session.mount('https://', adapter)
    return session

def get_currencyrate(plugin, currency, urlformat, replymembers):
    # NOTE: Bitstamp has a DNS/Proxy issues that can return 404
    # Workaround: retry up to 5 times with a delay
    currency_lc = currency.lower()
    url = urlformat.format(currency_lc=currency_lc, currency=currency)
    r = requests_retry_session(retries=5, status_forcelist=[404]).get(url, proxies=plugin.proxies)

    if r.status_code != 200:
        plugin.log(level='info', message='{}: bad response {}'.format(url, r.status_code))
        return None

    json = r.json()
    for m in replymembers:
        expanded = m.format(currency_lc=currency_lc, currency=currency)
        if expanded not in json:
            plugin.log(level='debug', message='{}: {} not in {}'.format(url, expanded, json))
            return None
        json = json[expanded]

    try:
        return Millisatoshi(int(10**11 / float(json)))
    except Exception:
        plugin.log(level='info', message='{}: could not convert {} to msat'.format(url, json))
        return None

def set_proxies(plugin):
    config = plugin.rpc.listconfigs()
    if 'always-use-proxy' in config and config['always-use-proxy']:
        paddr = config['proxy']
        # Default port in 9050
        if ':' not in paddr:
            paddr += ':9050'
        plugin.proxies = {'https': 'socks5h://' + paddr,
                          'http': 'socks5h://' + paddr}
    else:
        plugin.proxies = None

# Cache returns cached result if <60 seconds old.
# Stable Channels may not need
@cached(cache=TTLCache(maxsize=1024, ttl=60))
def get_rates(plugin, currency):
    rates = {}
    for s in sources:
        r = get_currencyrate(plugin, currency, s.urlformat, s.replymembers)
        if r is not None:
            rates[s.name] = r

    print("rates line 165",rates)
    return rates

@plugin.method("currencyconvert")
def currencyconvert(plugin, amount, currency):
    """Converts currency using given APIs."""
    rates = get_rates(plugin, currency.upper())
    if len(rates) == 0:
        raise Exception("No values available for currency {}".format(currency.upper()))

    val = statistics.median([m.millisatoshis for m in rates.values()]) * float(amount)
    
    estimated_price = "{:.2f}".format(100000000000 / statistics.median([m.millisatoshis for m in rates.values()]))

    return ({"msat": Millisatoshi(round(val))}, estimated_price)

# Section 3 - Core logic 


    # extract 

    # # we save to disk so that we don't get borked if the node restarts
    # # assumes notification calls are synchronous (not thread safe)
    # with open('moves.json', 'a') as f:
    #     f.write(json.dumps(coin_movement) + ',')


# This function is the scheduler, formatted to fire every 5 minutes
# This begins your regularly scheduled programming
def start_scheduler(sc):
    scheduler = BlockingScheduler()
    scheduler.add_job(check_stables, 'cron', minute='0/1', args=[sc])
    scheduler.start()

# 5 scenarios to handle
# Scenario 1 - Difference to small to worry about (under $0.01) = do nothing
# Scenario 2 - Node is stableReceiver and expects to get paid = wait 30 seconds; check on payment
# Scenario 3 - Node is stableProvider and needs to pay = keysend and exit
# Scenario 4 - Node is stableReceiver and needs to pay = keysend and exit
# Scenario 5 - Node is stableProvider and expects to get paid = wait 30 seconds; check on payment
# "sc" = "Stable Channel" object
def check_stables(sc):
    l1 = LightningRpc(sc.lightning_rpc_path)

    msat_dict, estimated_price = currencyconvert(plugin, sc.expected_dollar_amount, "USD")

    expected_msats = msat_dict["msat"]

    # Get channel data  
    list_funds_data = l1.listfunds()
    channels = list_funds_data.get("channels", [])
    
    # Find the correct stable channel and set balances
    for channel in channels:
        if channel.get("short_channel_id") == sc.short_channel_id:
            sc.our_balance = channel.get("our_amount_msat")
            sc.their_balance = Millisatoshi.__sub__(channel.get("amount_msat"), sc.our_balance)

    # Get Stable Receiver dollar amount
    if sc.is_stable_receiver:
        # subtract the native_amount_msat (regular BTC)
        sc.stable_receiver_dollar_amount = round((int(sc.our_balance - sc.native_amount_msat) * sc.expected_dollar_amount) / int(expected_msats), 3)
    else:
        sc.stable_receiver_dollar_amount = round((int(sc.their_balance - sc.native_amount_msat) * sc.expected_dollar_amount) / int(expected_msats), 3)

    formatted_time = datetime.utcnow().strftime("%H:%M %d %b %Y")
    
    sc.payment_made = False
    amount_too_small = False

    print("Line 239 check")
    print (sc.__str__())

    # Scenario 1 - Difference to small to worry about (under $0.01) = do nothing
    if abs(sc.expected_dollar_amount - float(sc.stable_receiver_dollar_amount)) < 0.01:
        amount_too_small = True
    else:
        # Round difference to nearest msat; we may need to pay it
        if sc.is_stable_receiver:
            may_need_to_pay_amount = round(abs(int(expected_msats) -  int(sc.our_balance)))
        else:
            may_need_to_pay_amount = round(abs(int(expected_msats) - int(sc.their_balance)))

    # USD price went down.
    if not amount_too_small and (sc.stable_receiver_dollar_amount < sc.expected_dollar_amount):
        # Scenario 2 - Node is stableReceiver and expects to get paid = wait 30 seconds; check on payment 
        if sc.is_stable_receiver:
            time.sleep(30)

            list_funds_data = l1.listfunds()

            # We should have payment now; check that amount is within 1 penny
            channels = list_funds_data.get("channels", [])
    
            for channel in channels:
                if channel.get("short_channel_id") == sc.short_channel_id:
                    new_our_stable_balance_msat = channel.get("our_amount_msat") - sc.native_amount_msat
                else:
                    print("Could not find channel")
                  
                new_stable_receiver_dollar_amount = round((int(new_our_stable_balance_msat) * sc.expected_dollar_amount) / int(expected_msats), 3)

            if sc.expected_dollar_amount - float(new_stable_receiver_dollar_amount) < 0.01:
                sc.payment_made = True
            else:
                # Increase risk score
                sc.risk_score = sc.risk_score + 1
            

        elif not(sc.is_stable_receiver):
            # Scenario 3 - Node is stableProvider and needs to pay = keysend and exit
            l1.keysend(sc.counterparty,may_need_to_pay_amount)
            
            # TODO - error handling
            sc.payment_made = True

    elif amount_too_small:
        sc.payment_made = False

    # USD price went up
    # TODO why isnt expected_dollar_amount being a float?
    elif not amount_too_small and sc.stable_receiver_dollar_amount > sc.expected_dollar_amount:
        # 4 - Node is stableReceiver and needs to pay = keysend
        if sc.is_stable_receiver:
            l1.keysend(sc.counterparty,may_need_to_pay_amount)
            
            # TODO - error handling
            sc.payment_made = True

        # Scenario 5 - Node is stableProvider and expects to get paid = wait 30 seconds; check on payment
        elif not(sc.is_stable_receiver):
            time.sleep(30)

            list_funds_data = l1.listfunds()

            channels = list_funds_data.get("channels", [])
    
            for channel in channels:
                if channel.get("short_channel_id") == sc.short_channel_id:
                    print("ok")
                else:
                    print("Could not find channel")
                # We should have payment now; check amount is within 1 penny
                new_our_balance = channel.get("our_amount_msat")
                new_their_stable_balance_msat = Millisatoshi.__sub__(channel.get("amount_msat"), new_our_balance) - sc.native_amount_msat

                new_stable_receiver_dollar_amount = round((int(new_their_stable_balance_msat) * sc.expected_dollar_amount) / int(expected_msats), 3)

            if sc.expected_dollar_amount - float(new_stable_receiver_dollar_amount) < 0.01:
                sc.payment_made = True
            else:
                # Increase risk score 
                sc.risk_score = sc.risk_score + 1

    # We write this to the main ouput file.
    json_line = f'{{"formatted_time": "{formatted_time}", "estimated_price": {estimated_price}, "expected_dollar_amount": {sc.expected_dollar_amount}, "stable_receiver_dollar_amount": {sc.stable_receiver_dollar_amount}, "payment_made": {sc.payment_made}, "risk_score": {sc.risk_score}}},\n'

    # Log the result
    # How to log better?
    if sc.is_stable_receiver:
        file_path = '/home/clightning/stablelog1.json'

        with open(file_path, 'a') as file:
            file.write(json_line)

    elif not(sc.is_stable_receiver):
        file_path = '/home/clightning/stablelog2.json'

        with open(file_path, 'a') as file:
            file.write(json_line)

# Section 4 - Plug-in initialization
@plugin.init()
def init(options, configuration, plugin):
    set_proxies(plugin)

    print(options['is-stable-receiver'])
    
    # Need to handle boolean input this way
    if options['is-stable-receiver'] == "False":
        is_stable_receiver = False
    elif options['is-stable-receiver'] == "True":
        is_stable_receiver = True

    print(is_stable_receiver)
    # convert to millsatoshis ...
    if int(options['native-btc-amount']) > 0:
        native_btc_amt_msat = int(options['native-btc-amount']) * 1000
    else:
        native_btc_amt_msat = 0

    sc = StableChannel(
            plugin=plugin,
            short_channel_id=options['short-channel-id'],
            expected_dollar_amount=float(options['stable-dollar-amount']),
            native_amount_msat=native_btc_amt_msat,
            is_stable_receiver=is_stable_receiver,
            counterparty=options['counterparty'],
            lightning_rpc_path=options['lightning-rpc-path'],
            our_balance=0,
            their_balance=0,
            risk_score=0,
            stable_receiver_dollar_amount=0,
            stable_provider_dollar_amount=0,
            timestamp=0,
            formatted_datetime='',
            payment_made=False
    )

    print("Starting Stable Channel with these details:")
    print(sc.short_channel_id)
    print(sc.expected_dollar_amount)
    print(sc.native_amount_msat)
    print(sc.counterparty)
    print(sc.lightning_rpc_path)

    # Need to start a new thread so init funciotn can return
    threading.Thread(target=start_scheduler, args=(sc,)).start()

plugin.add_option(name='short-channel-id', default='', description='Input the channel short-channel-id you wish to stabilize.')
plugin.add_option(name='is-stable-receiver', default='', description='Input True if you are the Stable Receiever; False if you are the Stable Provider.')
plugin.add_option(name='stable-dollar-amount', default='', description='Input the amount of dollars you want to keep stable.')
plugin.add_option(name='native-btc-amount', default='', description='Input the amount of bitcoin you do not want to be kept stable in dollar terms .. e.g. 0.0012btc. Include the btc at the end with no space.')
plugin.add_option(name='counterparty', default='', description='Input the nodeID of your counterparty.')
plugin.add_option(name='lightning-rpc-path', default='', description='Input your Lightning RPC path.')

plugin.run()
