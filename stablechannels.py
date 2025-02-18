#!/usr/bin/env python3

# This is the Core Lightning Python plug-in for Stable Channels - https://github.com/ElementsProject/lightning

# Stable Channels: p2p BTCUSD trading on Lightning
# Contents
# Section 1 - Dependencies and main data structure
# Section 2 - Price feed config and logic
# Section 3 - Core logic 
# Section 4 - Plug-in initialization

# Section 1 - Dependencies and main data structure
from pyln.client import Plugin # Library for CLN Python plug-ins created by Blockstream 
from pyln.client import Millisatoshi # Library for CLN Python plug-ins created by Blockstream 
from collections import namedtuple # Standard on Python 3
from cachetools import cached, TTLCache # Used to handle price feed calls; probably can remove
import requests # Standard on Python 3.7+
from requests.adapters import HTTPAdapter 
from requests.packages.urllib3.util.retry import Retry
import statistics # Standard on Python 3
import time # Standard on Python 3
import os
from datetime import datetime 
from apscheduler.schedulers.blocking import BlockingScheduler # Used to check balances every 5 minutes
import threading # Standard on Python 3

plugin = Plugin()

class StableChannel:
    def __init__(
        self,
        plugin: Plugin,
        channel_id: str,
        expected_dollar_amount: float,
        native_amount_msat: int,
        is_stable_receiver: bool,
        counterparty: str,
        our_balance: float,
        their_balance: float,
        risk_score: int,
        stable_receiver_dollar_amount: float,
        stable_provider_dollar_amount: float,
        timestamp: int,
        formatted_datetime: str,
        payment_made: bool,
        sc_dir: str
    ):
        self.plugin = plugin
        self.channel_id = channel_id
        self.expected_dollar_amount = expected_dollar_amount
        self.native_amount_msat = native_amount_msat
        self.is_stable_receiver = is_stable_receiver
        self.counterparty = counterparty
        self.our_balance = our_balance
        self.their_balance = their_balance
        self.risk_score = risk_score
        self.stable_receiver_dollar_amount = stable_receiver_dollar_amount
        self.stable_provider_dollar_amount = stable_provider_dollar_amount
        self.timestamp = timestamp
        self.formatted_datetime = datetime
        self.payment_made = payment_made
        self.sc_dir = sc_dir

    def __str__(self):
        return (
            f"StableChannel(\n"
            f"    channel_id={self.channel_id},\n"
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
            f"    sc_dir={self.sc_dir}\n"
            f")"
        )

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
    # Source('coindesk',
    #        'https://api.coindesk.com/v1/bpi/currentprice/{currency}.json',
    #        ['bpi', '{currency}', 'rate_float']),
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

    plugin.log(level="debug", message=f"rates line 165 {rates}")
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


def msats_to_currency(msats, rate_currency_per_btc):
    msats = float(msats) 
    rate_currency_per_btc = float(rate_currency_per_btc) 
    return msats / 1e11 * rate_currency_per_btc

# Section 3 - Core logic 

# This function is the scheduler, formatted to fire every 5 minutes
# This begins your regularly scheduled programming
def start_scheduler(plugin, sc):
    scheduler = BlockingScheduler()
    scheduler.add_job(check_stables, 'cron', minute='0/5', args=[plugin, sc])
    scheduler.start()

# 5 scenarios to handle
# Scenario 1 - Difference to small to worry about (under $0.01) = do nothing
# Scenario 2 - Node is stableReceiver and expects to get paid = wait 30 seconds; check on payment
# Scenario 3 - Node is stableProvider and needs to pay = keysend and exit
# Scenario 4 - Node is stableReceiver and needs to pay = keysend and exit
# Scenario 5 - Node is stableProvider and expects to get paid = wait 30 seconds; check on payment
# "sc" = "Stable Channel" object
def check_stables(plugin, sc):

    msat_dict, estimated_price = currencyconvert(plugin, sc.expected_dollar_amount, "USD")

    expected_msats = msat_dict["msat"]

    # Get channel data  
    list_funds_data = plugin.rpc.listfunds()
    channels = list_funds_data.get("channels", [])
    
    # Find the correct stable channel and set balances
    for channel in channels:
        if channel.get("channel_id") == sc.channel_id:
            sc.our_balance = channel.get("our_amount_msat")
            sc.their_balance = Millisatoshi.__sub__(channel.get("amount_msat"), sc.our_balance)

    # Get Stable Receiver dollar amount
    if sc.is_stable_receiver:
        sc.stable_receiver_dollar_amount = round((int((sc.our_balance - sc.native_amount_msat) * sc.expected_dollar_amount)) / int(expected_msats), 3)
    else:
        sc.stable_receiver_dollar_amount = round((int((sc.their_balance - sc.native_amount_msat) * sc.expected_dollar_amount)) / int(expected_msats), 3)

    formatted_time = datetime.utcnow().strftime("%H:%M %d %b %Y")
    
    sc.payment_made = False
    amount_too_small = False

    plugin.log (sc.__str__())

    # Scenario 1 - Difference to small to worry about (under $0.01) = do nothing
    if abs(sc.expected_dollar_amount - float(sc.stable_receiver_dollar_amount)) < 0.01:
        amount_too_small = True
    else:
        # Round difference to nearest msat; we may need to pay it
        if sc.is_stable_receiver:
            may_need_to_pay_amount = round(abs(int(expected_msats + sc.native_amount_msat) -  int(sc.our_balance)))
        else:
            may_need_to_pay_amount = round(abs(int(expected_msats + sc.native_amount_msat) - int(sc.their_balance)))

    # USD price went down.
    if not amount_too_small and (sc.stable_receiver_dollar_amount < sc.expected_dollar_amount):
        # Scenario 2 - Node is stableReceiver and expects to get paid = wait 30 seconds; check on payment 
        if sc.is_stable_receiver:
            time.sleep(30)

            list_funds_data = plugin.rpc.listfunds()

            # We should have payment now; check that amount is within 1 penny
            channels = list_funds_data.get("channels", [])
    
            for channel in channels:
                if channel.get("channel_id") == sc.channel_id:
                    plugin.log("Found Stable Channel")
                    new_our_stable_balance_msat = channel.get("our_amount_msat") - sc.native_amount_msat
                else:
                    plugin.log("Could not find channel")
                  
                new_stable_receiver_dollar_amount = round((int(new_our_stable_balance_msat) * sc.expected_dollar_amount) / int(expected_msats), 3)

            if sc.expected_dollar_amount - float(new_stable_receiver_dollar_amount) < 0.01:
                sc.payment_made = True
            else:
                # Increase risk score
                sc.risk_score = sc.risk_score + 1
            

        elif not(sc.is_stable_receiver):
            # Scenario 3 - Node is stableProvider and needs to pay = keysend and exit
            plugin.rpc.keysend(sc.counterparty,may_need_to_pay_amount)
            
            # TODO - error handling
            sc.payment_made = True

    elif amount_too_small:
        sc.payment_made = False

    # USD price went up
    # TODO why isnt expected_dollar_amount being a float?
    elif not amount_too_small and sc.stable_receiver_dollar_amount > sc.expected_dollar_amount:
        # 4 - Node is stableReceiver and needs to pay = keysend
        if sc.is_stable_receiver:
            plugin.rpc.keysend(sc.counterparty,may_need_to_pay_amount)
            
            # TODO - error handling
            sc.payment_made = True

        # Scenario 5 - Node is stableProvider and expects to get paid = wait 30 seconds; check on payment
        elif not(sc.is_stable_receiver):
            time.sleep(30)

            list_funds_data = plugin.rpc.listfunds()

            channels = list_funds_data.get("channels", [])
    
            for channel in channels:
                if channel.get("channel_id") == sc.channel_id:
                    plugin.log("Found Stable Channel")
                else:
                    plugin.log("Could not find Stable Channel")

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
    plugin.log(json_line)
    if sc.is_stable_receiver:
        file_path = os.path.join(sc.sc_dir, "stablelog1.json")

        with open(file_path, 'a') as file:
            file.write(json_line)

    elif not(sc.is_stable_receiver):
        file_path = os.path.join(sc.sc_dir, "stablelog2.json")

        with open(file_path, 'a') as file:
            file.write(json_line)

# this method updates the balances in memory
def handle_coin_movement(plugin, sc, *args, **kwargs):

    coin_movement = kwargs.get('coin_movement', {})
    version = coin_movement.get('version')
    node_id = coin_movement.get('node_id')
    type_ = coin_movement.get('type')
    account_id = coin_movement.get('account_id')
    payment_hash = coin_movement.get('payment_hash')
    part_id = coin_movement.get('part_id')
    credit_msat = coin_movement.get('credit_msat')
    debit_msat = coin_movement.get('debit_msat')
    fees_msat = coin_movement.get('fees_msat')
    tags = coin_movement.get('tags', [])
    timestamp = coin_movement.get('timestamp')
    coin_type = coin_movement.get('coin_type')

    # Print or manipulate the extracted values as needed
    plugin.log(f"Version:{version}")
    plugin.log(f"Node ID:{node_id}")
    plugin.log(f"Type:{type_}")
    plugin.log(f"Account ID:{account_id}")
    plugin.log(f"Payment Hash:{payment_hash}")
    plugin.log(f"Part ID:{part_id}")
    plugin.log(f"Credit Millisatoshi:{credit_msat}")
    plugin.log(f"Debit Millisatoshi:{debit_msat}")
    plugin.log(f"Fees Millisatoshi:{fees_msat}")
    plugin.log(f"Tags:{tags}")
    plugin.log(f"Timestamp:{timestamp}")
    plugin.log(f"Coin Type:{coin_type}")

    if sc.channel_id == account_id:
        # if a payment has been routed out of this account (channel)
        # then this means we are the Stable Provider
        # and we need to adjust the Stable Balance downwards
        if 'routed' in tags: 

            # the Stable Provider routed a pay out
            if credit_msat > 0:
                # need to convert msats to dollars
                plugin.log(f"previous stable dollar amount:{sc.expected_dollar_amount}")
                msat_dict, estimated_price = currencyconvert(plugin, sc.expected_dollar_amount, "USD")
                currency_units = msats_to_currency(int(credit_msat), estimated_price)
                sc.expected_dollar_amount -= currency_units
                plugin.log(f"estimated_price:{estimated_price}")
                plugin.log(f"post stable_dollar_amount:{sc.expected_dollar_amount}", )

            # the SR got paid
            if debit_msat > 0:
                plugin.log("shall debit, somehow")
                # sc.our_balance = sc.our_balance + credit_msat

        if 'invoice' in tags:
            # We need to check the payment destination is NOT the counterparty
            # Because CLN also records keysends as 'invoice'
            listpays_data =  plugin.rpc.listpays(payment_hash=payment_hash)

            if listpays_data and listpays_data["pays"]:
                destination = listpays_data["pays"][0]["destination"]

                # If the counterparty is not the destination,
                # Thne it is a payment out, probably made by Stable Receiver
                if sc.counterparty != destination:
            
                    # if the we are the dest then we received, increase expected dollar value
                    if credit_msat > 0:
                        plugin.log("shall credit somehow")

                        #sc.stable_dollar_amount += credit_msat

                    # the Stable Receiver paid an invoice
                    if debit_msat > 0:
                        plugin.log(f"previous stable dollar amount:{sc.expected_dollar_amount}")
                        msat_dict, estimated_price = currencyconvert(plugin, sc.expected_dollar_amount, "USD")
                        debit_plus_fees = debit_msat + fees_msat
                        currency_units = msats_to_currency(int(debit_plus_fees), estimated_price)
                        sc.expected_dollar_amount -= currency_units
                        plugin.log(f"estimated_price:{estimated_price}")
                        plugin.log(f"currency_units:{currency_units}")
                        plugin.log(f"post stable_dollar_amount:{sc.expected_dollar_amount}")
                        # sc.our_balance = sc.our_balance + credit_msat

def parse_boolean(value):
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        value_lower = value.strip().lower()
        if value_lower in {'true', 'yes', '1'}:
            return True
        elif value_lower in {'false', 'no', '0'}:
            return False
    raise ValueError(f"Invalid boolean value: {value}")

@plugin.method("dev-check-stable")
def dev_check_stable(plugin):
    # immediately run check_stable, but only if we are a dev
    # this can be used in tests and maybe to pass artificial currency rate changes
    dev = plugin.rpc.listconfigs("developer")["configs"]["developer"]["set"]
    if dev:
        check_stables(plugin, sc)
        return {"result": "OK"}
    else:
        raise Exception("ERROR: not a --developer")

# Section 4 - Plug-in initialization
@plugin.init()
def init(options, configuration, plugin):
    set_proxies(plugin)

    plugin.log(level="debug", message=options['is-stable-receiver'])
    
    # Need to handle boolean input this way
    is_stable_receiver = parse_boolean(options['is-stable-receiver'])

    # convert to millsatoshis ...
    if int(options['native-btc-amount']) > 0:
        native_btc_amt_msat = int(options['native-btc-amount']) * 1000
    else:
        native_btc_amt_msat = 0

    lightning_dir = plugin.rpc.getinfo()["lightning-dir"]
    sc_dir = os.path.join(lightning_dir, "stablechannels")
    os.makedirs(sc_dir, exist_ok=True)

    global sc
    sc = StableChannel(
            plugin=plugin,
            channel_id=options['channel-id'],
            expected_dollar_amount=float(options['stable-dollar-amount']),
            native_amount_msat=native_btc_amt_msat,
            is_stable_receiver=is_stable_receiver,
            counterparty=options['counterparty'],
            our_balance=0,
            their_balance=0,
            risk_score=0,
            stable_receiver_dollar_amount=0,
            stable_provider_dollar_amount=0,
            timestamp=0,
            formatted_datetime='',
            payment_made=False,
            sc_dir=sc_dir
    )

    plugin.log("Starting Stable Channel with these details:")
    plugin.log(sc.__str__())

    # Need to start a new thread so init funciotn can return
    threading.Thread(target=start_scheduler, args=(plugin, sc)).start()

plugin.add_option(name='channel-id', default='', description='Input the channel ID you wish to stabilize.')
plugin.add_option(name='is-stable-receiver', default='', description='Input True if you are the Stable Receiever; False if you are the Stable Provider.')
plugin.add_option(name='stable-dollar-amount', default='', description='Input the amount of dollars you want to keep stable.')
plugin.add_option(name='native-btc-amount', default='', description='Input the amount of bitcoin you do not want to be kept stable, in sats.')
plugin.add_option(name='counterparty', default='', description='Input the nodeID of your counterparty.')

plugin.add_subscription("coin_movement", lambda *args, **kwargs: handle_coin_movement(plugin, sc, *args, **kwargs))

plugin.run()
