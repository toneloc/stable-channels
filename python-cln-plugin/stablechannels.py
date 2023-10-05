
# PeerStables: p2p USD stable channels on Lightning

"""
╔═╗┌─┐┌─┐┬─┐╔═╗┌┬┐┌─┐┌┐ ┬  ┌─┐┌─┐  
╠═╝├┤ ├┤ ├┬┘╚═╗ │ ├─┤├┴┐│  ├┤ └─┐  
╩  └─┘└─┘┴└─╚═╝ ┴ ┴ ┴└─┘┴─┘└─┘└─┘                                                       
"""

from pyln.client import Plugin
from collections import namedtuple
from pyln.client import Millisatoshi
from cachetools import cached, TTLCache
from requests.adapters import HTTPAdapter
from requests.packages.urllib3.util.retry import Retry
import requests
import statistics
from pyln.client import LightningRpc
import random 
from threading import Timer
import os
import time
import datetime
import pytz

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
        deliquency_meter: int,
        stable_receiver_dollar_amount: float,   # New attribute
        stable_provider_dollar_amount: float    # New attribute
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
        self.deliquency_meter = deliquency_meter
        self.stable_receiver_dollar_amount = stable_receiver_dollar_amount
        self.stable_provider_dollar_amount = stable_provider_dollar_amount

    def print_values(self):
        print("Plugin:", self.plugin)
        print("Short Channel id:", self.short_channel_id)
        print("Expected Dollar Amount:", self.expected_dollar_amount)
        print("Minimum Margin Ratio:", self.minimum_margin_ratio)
        print("Is Stable Receiver:", self.is_stable_receiver)
        print("Counterparty:", self.counterparty)
        print("Lightning RPC path", self.lightning_rpc_path)
        print("Our Balance:", self.our_balance)
        print("Their Balance:", self.their_balance)
        print("Deliquency Meter:", self.deliquency_meter)
        print("Stable Receiver Dollar Amount:", self.stable_receiver_dollar_amount)
        print("Stable Provider Dollar Amount:", self.stable_provider_dollar_amount)

Source = namedtuple('Source', ['name', 'urlformat', 'replymembers'])

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

    return rates

##############
@plugin.method("listalltransactions")
def listalltransactions():
    l1 = LightningRpc("/home/ubuntu/.lightning/testnet/lightning-rpc")
    balances = l1.listtransactions()
    print(balances)
    return balances

@plugin.method("printinvoice")
def printinvoice():
    """Prints invoice."""
    l1 = LightningRpc("/home/ubuntu/.lightning/testnet/lightning-rpc")
    invoice = l1.invoice(100, "lbl{}".format(random.random()), "testpayment")
    print("invoice")
    print(invoice)
    return invoice

    # return get_rates(plugin, cu

@plugin.method("stablesendcustommmsg")
def stablesendcustommmsg(plugin):
    """Gets currency from given APIs."""
    l1 = LightningRpc("/home/ubuntu/.lightning/testnet/lightning-rpc")
    result = l1.sendcustommsg("030d21990f4c6394165aabd43e793ea572b822fa33c2fd2c7f9b406315e191234c","0X68656C6C6F")
    return result

@plugin.method("acceptstable")
def acceptstable(plugin, currency):
    return {"hello!" : 2}
    # return get_rates(plugin, currency.upper())

@plugin.method("currencyconvert")
def currencyconvert(plugin, amount, currency):
    """Converts currency using given APIs."""
    rates = get_rates(plugin, currency.upper())
    if len(rates) == 0:
        raise Exception("No values available for currency {}".format(currency.upper()))
    val = statistics.median([m.millisatoshis for m in rates.values()]) * float(amount)
    return {"msat": Millisatoshi(round(val))}

# 5 scenarios
# 1 - amount to small to worry about = do nothing
# 2 - node is stableReceiver and needs to get paid = enter expect payment loop
# 3 - node is stableProvider and needs to get paid = enter expect payment loop
# 4 - node is stableReceiver and needs to pay = pay
# 5 - mode is stableProvider and needs to pay = pay
def check_stables(stable_channel):
    print(stable_channel.is_stable_receiver)
    l1 = LightningRpc(stable_channel.lightning_rpc_path)

    expected_msats = currencyconvert(plugin, stable_channel.expected_dollar_amount, "USD")['msat']

    # ensure connected ... 
    print(l1.connect(stable_channel.counterparty))
    user_funds_data = l1.listfunds()
    channels = user_funds_data.get("channels", [])
    
    for channel in channels:
        if channel.get("short_channel_id") == stable_channel.short_channel_id:
            stable_channel.our_balance = channel.get("our_amount_msat")
            stable_channel.their_balance = Millisatoshi.__sub__(channel.get("amount_msat"), stable_channel.our_balance)
            print("Our balance.................... = " + str(stable_channel.our_balance))
            print("Their balance.................. = " + str(stable_channel.their_balance))
   

    if stable_channel.is_stable_receiver:
        stable_channel.stable_receiver_dollar_amount = round((int(stable_channel.our_balance) * stable_channel.expected_dollar_amount) / int(expected_msats), 3)
    else:
        stable_channel.stable_receiver_dollar_amount = round((int(stable_channel.their_balance) * stable_channel.expected_dollar_amount) / int(expected_msats), 3)

     # Set the timezone to Eastern Time (ET)
    eastern_timezone = pytz.timezone('US/Eastern')

    # Get the current time in the Eastern Time zone
    current_time_et = datetime.datetime.now(eastern_timezone)

    # Format the current time prettily
    formatted_time = current_time_et.strftime('%A, %B %d, %Y %I:%M:%S %p %Z')

    # Print the formatted time
    print("Time = " + formatted_time)
    print("Is stable receiver?......     = " + str(stable_channel.is_stable_receiver))
    print("Stable asset.............     = USD")
    print("Expected stable reciever amt  = " + str(stable_channel.expected_dollar_amount))
    print("Current stable receiver amt   = " + str(stable_channel.stable_receiver_dollar_amount))
    print("Minimum margin ratio........  = " + str(stable_channel.minimum_margin_ratio))
    # print("Current margin ratio........ = " + str("TODO"))
    print("Expected stable receiver msats= " + str(expected_msats))
    

    if stable_channel.is_stable_receiver:
        print("Current stable receiver msats = " + str(stable_channel.our_balance))
    else:
        print("Current stable receiver msats = " + str(stable_channel.their_balance))


    amount_too_small = False

    if abs(stable_channel.expected_dollar_amount - float(stable_channel.stable_receiver_dollar_amount)) < 0.01:
        amount_too_small = True
        print("Difference too small for payment.")
        print("")
    else:
        # Round to nearest msat
        if stable_channel.is_stable_receiver:
            may_need_to_pay_amount = round(abs(int(expected_msats) -  int(stable_channel.our_balance)))
        else:
            may_need_to_pay_amount = round(abs(int(expected_msats) - int(stable_channel.their_balance)))

    # USD price went down.
    if not amount_too_small and (stable_channel.stable_receiver_dollar_amount < stable_channel.expected_dollar_amount):
        # Scenario 1 - is stable receiver
        if stable_channel.is_stable_receiver:
            print("As Stable Receiver, I expect to GET PAID.")
            # TODO add expect payment loop

        # Scenario 2 - is stable provider
        elif not(stable_channel.is_stable_receiver):
            print("As Stable Provider, I expect to PAY node ID = " + stable_channel.counterparty)
            result = l1.keysend(stable_channel.counterparty,may_need_to_pay_amount)
            print(result)

    # Scenario 3
    elif amount_too_small:
        print("Price unchanged or difference too small; no payment needed.")

    # USD price went up
    # TODO why isnt expected_dollar_amount being a float?
    elif not amount_too_small and stable_channel.stable_receiver_dollar_amount > stable_channel.expected_dollar_amount:
        # Scenario 4
        if stable_channel.is_stable_receiver:
            print("As Stable Receiver, I expect to PAY + " + stable_channel.counterparty)
            result = l1.keysend(stable_channel.counterparty,may_need_to_pay_amount)
            print(result)

        # Scenario 5
        elif not(stable_channel.is_stable_receiver):
            print("As Stable Provider, I expect to GET PAID.")
            # do_expect_payment_loop(need_to_pay_amount + our_balance)


    Timer(10, check_stables, args=[stable_channel]).start()

@plugin.init()
def init(options, configuration, plugin):
    set_proxies(plugin)
    sourceopts = options['stable-details']

    # TODO pass in as args or add to lightning conf file instead?
    if sourceopts != ['']:
        for s in sourceopts:
            parts = s.split(',')
            
            if len(parts) != 6:
                raise Exception("Too few or too many paramaters at start")

            if parts[3] == "False":
                is_stable_receiver = False
            elif parts[3] == "True":
                is_stable_receiver = True

            stable_channel = StableChannel(
                plugin=plugin, 
                short_channel_id=parts[0],  
                expected_dollar_amount=float(parts[1]), 
                minimum_margin_ratio=float(parts[2]),
                is_stable_receiver=is_stable_receiver,  
                counterparty=parts[4],
                lightning_rpc_path=parts[5],
                our_balance=0,
                their_balance=0,
                deliquency_meter=0,
                stable_receiver_dollar_amount=0,
                stable_provider_dollar_amount=0
            )

        stable_channel.print_values()


    plugin.state_file = os.path.join(configuration['lightning-dir'],
                                     "persistent-channels.json")

    time.sleep(40)
    check_stables(stable_channel)

# As a bad example: binance,https://api.binance.com/api/v3/ticker/price?symbol=BTC{currency}T,price
plugin.add_option(name='stable-details', default='', description='Add source name,urlformat,resultmembers...')

# This has an effect only for recent pyln versions (0.9.3+).
plugin.options['stable-details']['multi'] = True

plugin.run()
