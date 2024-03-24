#!/usr/bin/python3

# Stable Channels: p2p BTCUSD trading on Lightning
# Contents
# Section 1 - Dependencies and main data structure
# Section 2 - Price feed config and logic
# Section 3 - Core logic 
# Section 4 - Initialization

# Section 1 - Dependencies and main data structure
from cachetools import cached, TTLCache # Used to handle price feed calls; probably can remove
import requests # Standard on Python 3.7+
from requests.adapters import HTTPAdapter 
from collections import namedtuple 
from requests.packages.urllib3.util.retry import Retry
import statistics # Standard on Python 3
import time # Standard on Python 3
from datetime import datetime 
from apscheduler.schedulers.blocking import BlockingScheduler # Used to check balances every 5 minutes
import threading # Standard on Python 3
import argparse

# Section 1 Dependencies and main data structure
class StableChannel:
    def __init__(
        self,
        short_channel_id: str,
        expected_dollar_amount: float,
        nonstable_amount_msat: int,
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
        macaroon_hex: str,
        tls_cert_path: str
    ):
        self.short_channel_id = short_channel_id
        self.expected_dollar_amount = expected_dollar_amount
        self.nonstable_amount_msat = nonstable_amount_msat
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
        self.macaroon_hex = macaroon_hex
        self.tls_cert_path = tls_cert_path

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

def get_currencyrate(currency, urlformat, replymembers):
    # NOTE: Bitstamp has a DNS/Proxy issues that can return 404
    # Workaround: retry up to 5 times with a delay
    currency_lc = currency.lower()
    url = urlformat.format(currency_lc=currency_lc, currency=currency)
    r = requests_retry_session(retries=5, status_forcelist=[404]).get(url)

    if r.status_code != 200:
        # plugin.log(level='info', message='{}: bad response {}'.format(url, r.status_code))
        return None

    json = r.json()
    for m in replymembers:
        expanded = m.format(currency_lc=currency_lc, currency=currency)
        if expanded not in json:
            # plugin.log(level='debug', message='{}: {} not in {}'.format(url, expanded, json))
            return None
        json = json[expanded]

    try:
        return int(10**11 / float(json))
    except Exception:
       print(" could not convert to sat'.format(url, json))")
       return None

def set_proxies():
    return
    # config = plugin.rpc.listconfigs()
    # if 'always-use-proxy' in config and config['always-use-proxy']:
    #     paddr = config['proxy']
    #     # Default port in 9050
    #     if ':' not in paddr:
    #         paddr += ':9050'
    #     plugin.proxies = {'https': 'socks5h://' + paddr,
    #                       'http': 'socks5h://' + paddr}
    # else:
    #     plugin.proxies = None

# Cache returns cached result if <60 seconds old.
# Stable Channels may not need
@cached(cache=TTLCache(maxsize=1024, ttl=60))
def get_rates(currency):
    rates = {}
    for s in sources:
        r = get_currencyrate(currency, s.urlformat, s.replymembers)
        if r is not None:
            rates[s.name] = r

    print("msats per dollar from exchanges: ",rates)
    return rates

def currencyconvert(amount, currency):
    """Converts currency using given APIs."""
    rates = get_rates(currency.upper())
    if len(rates) == 0:
        raise Exception("No values available for currency {}".format(currency.upper()))

    val = statistics.median([m for m in rates.values()]) * float(amount)
    
    estimated_price = "{:.2f}".format(100000000000 / statistics.median([m for m in rates.values()]))

    return ({"msat": round(val)}, estimated_price)


# Section 3 - Core logic 

# This function is the scheduler, formatted to fire every 5 minutes
# Regularly scheduled programming
def start_scheduler(sc):
    scheduler = BlockingScheduler()
    scheduler.add_job(check_stables, 'cron', minute='0/5', args=[sc])
    scheduler.start()

# 5 scenarios to handle
# Scenario 1 - Difference to small to worry about (under $0.01) = do nothing
# Scenario 2 - Node is stableReceiver and expects to get paid = wait 30 seconds; check on payment
# Scenario 3 - Node is stableProvider and needs to pay = keysend and exit
# Scenario 4 - Node is stableReceiver and needs to pay = keysend and exit
# Scenario 5 - Node is stableProvider and expects to get paid = wait 30 seconds; check on payment
# "sc" = "Stable Channel" object
def check_stables(sc):
    msat_dict, estimated_price = currencyconvert(sc.expected_dollar_amount, "USD")

    expected_msats = msat_dict["msat"]

    currencyconvert(100, "USD")

def main():
    parser = argparse.ArgumentParser(description='LND Script Arguments')
    parser.add_argument('--lnd-server', type=str, required=True, help='LND server address')
    parser.add_argument('--macaroon-hex', type=str, required=True, help='Hex-encoded macaroon for authentication')
    parser.add_argument('--tls-cert-path', type=str, required=True, help='TLS cert path for auth to server for authentication')
    parser.add_argument('--expected-dollar-amount', type=float, required=True, help='Expected dollar amount')
    parser.add_argument('--short-channel-id', type=str, required=True, help='Short channel ID')
    parser.add_argument('--native-amount-sat', type=float, required=True, help='Native amount in msat')
    parser.add_argument('--is-stable-receiver', type=lambda x: (str(x).lower() == 'true'), required=True, help='Is stable receiver flag')
    parser.add_argument('--counterparty', type=str, required=True, help='LN Node ID of counterparty')

    args = parser.parse_args()

    # Path to your TLS certificate
    # tls_cert_path = '/Users/t/.polar/networks/8/volumes/lnd/alice/tls.cert'

    # Macaroon in hexadecimal string format
    # macaroon_hex = '0201036c6e6402f801030a10d640b0094ad88e5a9902ce8af2e9f1251201301a160a0761646472657373120472656164120577726974651a130a04696e666f120472656164120577726974651a170a08696e766f69636573120472656164120577726974651a210a086d616361726f6f6e120867656e6572617465120472656164120577726974651a160a076d657373616765120472656164120577726974651a170a086f6666636861696e120472656164120577726974651a160a076f6e636861696e120472656164120577726974651a140a057065657273120472656164120577726974651a180a067369676e6572120867656e657261746512047265616400000620985ae14816370b22f44a3543ebc92ebef19e1197138b365b9f349b5bc072acb2'

    sc = StableChannel(
        short_channel_id=args.short_channel_id, # Argparse autoconverts hyphens to undersores for Python
        expected_dollar_amount=args.expected_dollar_amount,
        nonstable_amount_msat=int(args.native_amount_sat * 1000),
        is_stable_receiver=args.is_stable_receiver,
        counterparty=args.counterparty,
        their_balance=0,
        risk_score=0,
        stable_receiver_dollar_amount=0,
        stable_provider_dollar_amount=0,
        timestamp=0,
        formatted_datetime='',
        payment_made=False,
        macaroon_hex='0201036c6e6402f801030a10d640b0094ad88e5a9902ce8af2e9f1251201301a160a0761646472657373120472656164120577726974651a130a04696e666f120472656164120577726974651a170a08696e766f69636573120472656164120577726974651a210a086d616361726f6f6e120867656e6572617465120472656164120577726974651a160a076d657373616765120472656164120577726974651a170a086f6666636861696e120472656164120577726974651a160a076f6e636861696e120472656164120577726974651a140a057065657273120472656164120577726974651a180a067369676e6572120867656e657261746512047265616400000620985ae14816370b22f44a3543ebc92ebef19e1197138b365b9f349b5bc072acb2',
        tls_cert_path = args.tls_cert_path
    )

    print(sc)
   

    # The URL you're making the request to
    url = 'https://127.0.0.1:8081/v1/channels'

    # Setup the request headers with the macaroon
    headers = {
        'Grpc-Metadata-macaroon': sc.macaroon_hex
    }

    # Make the request using the requests library. Verify parameter is path to TLS cert for SSL verification
    response = requests.get(url, headers=headers, verify=sc.tls_cert_path)

    print(response)
    # Check for HTTP codes other than 200
    if response.status_code != 200:
        print(f'Failed to fetch data: HTTP {response.status_code}')
    else:
        print('Success!')
        print(response.json())  # Assuming the response is JSON formatted

    currencyconvert(100, "USD")

if __name__ == "__main__":
    main()



# python3 lnd.py --lnd-server=127.0.0.1:10001 --lnd-cert=/Users/t/.polar/networks/8/volumes/lnd/alice/tls.cert --macaroon-hex= --expected-dollar-amount=100 --short-channel-id=0x123 --native-amount-msat=0.1 --is-stable-receiver=True --counterparty=0xABC --macaroon-hex=0201036c6e6402f801030a10d640b0094ad88e5a9902ce8af2e9f1251201301a160a0761646472657373120472656164120577726974651a130a04696e666f120472656164120577726974651a170a08696e766f69636573120472656164120577726974651a210a086d616361726f6f6e120867656e6572617465120472656164120577726974651a160a076d657373616765120472656164120577726974651a170a086f6666636861696e120472656164120577726974651a160a076f6e636861696e120472656164120577726974651a140a057065657273120472656164120577726974651a180a067369676e6572120867656e657261746512047265616400000620985ae14816370b22f44a3543ebc92ebef19e1197138b365b9f349b5bc072acb2
