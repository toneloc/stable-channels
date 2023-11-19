#!/usr/bin/python3

# PeerStables: p2p BTCUSD trading on Lightning

from pyln.client import Plugin
from collections import namedtuple
from pyln.client import Millisatoshi
from cachetools import cached, TTLCache
from requests.adapters import HTTPAdapter
from requests.packages.urllib3.util.retry import Retry
import requests
import statistics
from pyln.client import LightningRpc
import time
from datetime import datetime
from apscheduler.schedulers.blocking import BlockingScheduler
import threading

plugin = Plugin()

class StableChannel:
    def __init__(
        self,
        plugin: Plugin,
        short_channel_id: str,
        expected_dollar_amount: float,
        minimum_margin_ratio: float,
        is_stable_receiver: bool,
        counterparty: str,
        lightning_rpc_path: str,
        our_balance: float,
        their_balance: float,
        delinquency_meter: int,
        stable_receiver_dollar_amount: float,
        stable_provider_dollar_amount: float,
        timestamp: int,
        formatted_datetime: str,
        payment_made: bool
    ):
        self.plugin = plugin
        self.short_channel_id = short_channel_id
        self.expected_dollar_amount = expected_dollar_amount
        self.minimum_margin_ratio = minimum_margin_ratio
        self.is_stable_receiver = is_stable_receiver
        self.counterparty = counterparty
        self.lightning_rpc_path = lightning_rpc_path
        self.our_balance = our_balance
        self.their_balance = their_balance
        self.delinquency_meter = delinquency_meter
        self.stable_receiver_dollar_amount = stable_receiver_dollar_amount
        self.stable_provider_dollar_amount = stable_provider_dollar_amount
        self.timestamp = timestamp
        self.formatted_datetime = datetime
        self.payment_made = payment_made

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

# Don't grab these more than once per minute.
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

def start_scheduler(sc):
    # Now, enter into regularly scheduled programming
    # Schedule the check balances every 1 minute 
    scheduler = BlockingScheduler()
    scheduler.add_job(check_stables, 'cron', minute='0/1', args=[sc])
    scheduler.start()

# 5 scenarios to handle
# Scenario 1 - Difference to small to worry about = do nothing
# Scenario 2 - Node is stableReceiver and needs to get paid = wait 60 seconds; check on payment
# Scenario 3 - Node is stableProvider and needs to pay = keysend
# Scenario 4 - Node is stableReceiver and needs to pay = keysend
# Scenario 5 - Node is stableProvider and expects to get paid
# "sc" = "Stable Channel" object
def check_stables(sc):
    l1 = LightningRpc(sc.lightning_rpc_path)

    msat_dict, estimated_price = currencyconvert(plugin, sc.expected_dollar_amount, "USD")

    expected_msats = msat_dict["msat"]

    # Ensure we are connected
    list_funds_data = l1.listfunds()
    channels = list_funds_data.get("channels", [])
    
    # Find the correct stable channel
    for channel in channels:
        if channel.get("short_channel_id") == sc.short_channel_id:
            sc.our_balance = channel.get("our_amount_msat")
            sc.their_balance = Millisatoshi.__sub__(channel.get("amount_msat"), sc.our_balance)

    # Get Stable Receiver dollar amount
    if sc.is_stable_receiver:
        sc.stable_receiver_dollar_amount = round((int(sc.our_balance) * sc.expected_dollar_amount) / int(expected_msats), 3)
    else:
        sc.stable_receiver_dollar_amount = round((int(sc.their_balance) * sc.expected_dollar_amount) / int(expected_msats), 3)

    formatted_time = datetime.utcnow().strftime("%H:%M %d %b %Y")
    
    amount_too_small = False

    # 1 - Difference to small to worry about = do nothing
    if abs(sc.expected_dollar_amount - float(sc.stable_receiver_dollar_amount)) < 0.01:
        amount_too_small = True
    else:
        # Round to nearest msat
        if sc.is_stable_receiver:
            may_need_to_pay_amount = round(abs(int(expected_msats) -  int(sc.our_balance)))
        else:
            may_need_to_pay_amount = round(abs(int(expected_msats) - int(sc.their_balance)))

    # USD price went down.
    if not amount_too_small and (sc.stable_receiver_dollar_amount < sc.expected_dollar_amount):
        # Scenario 2 - Node is stableReceiver and needs to get paid 
        # Wait 30 seconds
        if sc.is_stable_receiver:
            time.sleep(30)

            # We should have payment now; check amount is within 1 penny
            sc.our_balance = channel.get("our_amount_msat")
            sc.their_balance = Millisatoshi.__sub__(channel.get("amount_msat"), sc.our_balance)

            sc.stable_receiver_dollar_amount = round((int(sc.our_balance) * sc.expected_dollar_amount) / int(expected_msats), 3)

            if abs(sc.expected_dollar_amount - float(sc.stable_receiver_dollar_amount)) < 0.01:
                sc.payment_made = True
            else:
                # Delinquecny. Increase delinquency meter
                sc.delinquency_meter = sc.delinquency_meter + 1
            

        elif not(sc.is_stable_receiver):
            # 3 - Node is stableProvider and needs to pay = keysend
            result = l1.keysend(sc.counterparty,may_need_to_pay_amount)
            
            # TODO - error handling
            sc.payment_made = True

    elif amount_too_small:
        sc.payment_made = False

    # USD price went up
    # TODO why isnt expected_dollar_amount being a float?
    elif not amount_too_small and sc.stable_receiver_dollar_amount > sc.expected_dollar_amount:
        # 4 - Node is stableReceiver and needs to pay = keysend
        if sc.is_stable_receiver:
            result = l1.keysend(sc.counterparty,may_need_to_pay_amount)
            
            # TODO - error handling
            sc.payment_made = True

        # Scenario 5 - Node is stableProvider and expects to get paid
        elif not(sc.is_stable_receiver):
            time.sleep(30)

            # We should have payment now; check amount is within 1 penny
            sc.our_balance = channel.get("our_amount_msat")
            sc.their_balance = Millisatoshi.__sub__(channel.get("amount_msat"), sc.our_balance)

            sc.stable_receiver_dollar_amount = round((int(sc.their_balance) * sc.expected_dollar_amount) / int(expected_msats), 3)

            if abs(sc.expected_dollar_amount - float(sc.stable_receiver_dollar_amount)) < 0.01:
                sc.payment_made = True
            else:
                # Delinquecny. Increase delinquency meter
                sc.delinquency_meter = sc.delinquency_meter + 1

    json_line = f'{{"formatted_time": "{formatted_time}", "estimated_price": {estimated_price}, "expected_dollar_amount": {sc.expected_dollar_amount}, "stable_receiver_dollar_amount": {sc.stable_receiver_dollar_amount}, "payment_made": {sc.payment_made}, "delinquency_meter": {sc.delinquency_meter}}},\n'

    # Log the result
    if sc.is_stable_receiver:
        file_path = '/home/ubuntu/stablelog1.csv'

        with open(file_path, 'a') as file:
            file.write(json_line)

    elif not(sc.is_stable_receiver):
        file_path = '/home/ubuntu/stablelog2.csv'

        with open(file_path, 'a') as file:
            file.write(json_line)


@plugin.init()
def init(options, configuration, plugin):
    print("here")
    set_proxies(plugin)
    stable_details = options['stable-details']

    print(str(stable_details))

    # TODO - Pass in as plugin start args
    if stable_details != ['']:
        for s in stable_details:
            parts = s.split(',')
            
            if len(parts) != 6:
                raise Exception("Too few or too many Stable Channel paramaters at start.")

            if parts[3] == "False":
                is_stable_receiver = False
            elif parts[3] == "True":
                is_stable_receiver = True

            sc = StableChannel(
                plugin=plugin, 
                short_channel_id=parts[0],  
                expected_dollar_amount=float(parts[1]), 
                minimum_margin_ratio=float(parts[2]),
                is_stable_receiver=is_stable_receiver,  
                counterparty=parts[4],
                lightning_rpc_path=parts[5],
                our_balance=0,
                their_balance=0,
                delinquency_meter=0,
                stable_receiver_dollar_amount=0,
                stable_provider_dollar_amount=0,        
                timestamp=0,
                formatted_datetime='',
                payment_made=False
            )

    # Let lightningd sync up before starting the stable tests
    time.sleep(15)

    # And ensure connected
    l1 = LightningRpc(sc.lightning_rpc_path)
    assert(l1.connect(sc.counterparty))

    # need to start a new thread so init funciotn can return
    threading.Thread(target=start_scheduler, args=(sc,)).start()
    
plugin.add_option(name='stable-details', default='', description='Input stable details.')

# This has an effect only for recent pyln versions (0.9.3+).
plugin.options['stable-details']['multi'] = True

plugin.run()

