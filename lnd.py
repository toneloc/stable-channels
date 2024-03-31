#!/usr/bin/python3

# Stable Channels: p2p BTCUSD trading on Lightning
# Contents
# Section 1 - Dependencies and main data structure
# Section 2 - Price feed config and logic
# Section 3 - Core logic 
# Section 4 - Initialization


# Section 1 - Dependencies and main data structure
# Dependencies
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
import codecs # Encodes macaroon as hex
from hashlib import sha256
from secrets import token_hex
import base64

# Main data structure
class StableChannel:
    def __init__(
        self,
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
        lnd_server_url: str,
        macaroon_hex: str,
        tls_cert_path: str


    ):
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
        self.lnd_server_url = lnd_server_url
        self.macaroon_hex = macaroon_hex
        self.tls_cert_path = tls_cert_path

    def __str__(self):
        return (f"StableChannel(channel_id={self.channel_id}, "
                f"native_amount_msat={self.native_amount_msat}, "
                f"expected_dollar_amount={self.expected_dollar_amount}, "
                f"is_stable_receiver={self.is_stable_receiver}, "
                f"counterparty={self.counterparty}, "
                f"our_balance={self.our_balance}, "
                f"their_balance={self.their_balance}, "
                f"risk_score={self.risk_score}, "
                f"stable_receiver_dollar_amount={self.stable_receiver_dollar_amount}, "
                f"stable_provider_dollar_amount={self.stable_provider_dollar_amount}, "
                f"timestamp={self.timestamp}, "
                f"formatted_datetime={self.formatted_datetime}, "
                f"payment_made={self.payment_made}, "
                f"lnd_server_url={self.lnd_server_url}, "
                f"macaroon_hex={self.macaroon_hex}, "
                f"tls_cert_path={self.tls_cert_path})")


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
# Helper functions

def b64_hex_transform(plain_str: str) -> str:
    """Returns the b64 transformed version of a hex string"""
    a_string = bytes.fromhex(plain_str)
    return base64.b64encode(a_string).decode()

def get_channel_info(sc):
    url = sc.lnd_server_url + '/v1/channels'
    headers = {'Grpc-Metadata-macaroon': sc.macaroon_hex}
    response = requests.get(url, headers=headers, verify=sc.tls_cert_path)
    return response.json()

def update_our_and_their_balance(sc, channels_data):
    for channel in channels_data['channels']:
        if channel['chan_id'] == sc.channel_id:
            sc.our_balance = int(channel['local_balance']) * 1000
            sc.their_balance = int(channel['remote_balance']) * 1000
            return
    print("Could not find channel")

def b64_transform(plain_str: str) -> str:
    """Returns the b64 transformed version of a string"""
    return base64.b64encode(plain_str.encode()).decode()

# This function is the scheduler, formatted to fire every 5 minutes
def start_scheduler(sc):
    scheduler = BlockingScheduler()
    scheduler.add_job(check_stables, 'cron', minute='0/1', args=[sc])
    scheduler.start()
    pass

def calculate_stable_receiver_dollar_amount(sc, balance, expected_msats):
    return round((int(balance - sc.native_amount_msat) * sc.expected_dollar_amount) / int(expected_msats), 3)

def keysend_payment(sc, amount_msat):
    dest = b64_hex_transform(sc.counterparty)
    pre_image = token_hex(32)
    payment_hash = sha256(bytes.fromhex(pre_image)).hexdigest()
    dest_custom_records = {
        5482373484: b64_hex_transform(pre_image),
        34349334: b64_transform("yoo"),
    }
    url = sc.lnd_server_url + '/v1/channels/transactions'
    headers = {'Grpc-Metadata-macaroon': sc.macaroon_hex}

    data = {
        "dest": dest,
        "amt": int(amount_msat / 1000),
        "payment_hash": b64_hex_transform(payment_hash),
        "dest_custom_records": dest_custom_records,
    }
    print("")
    print(str(data))
    response = requests.post(url=url, headers=headers, json=data, verify=sc.tls_cert_path)
    return response

# Core logic
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
    print("expected msats")
    print(expected_msats)

    channels_data = get_channel_info(sc)
    print(channels_data)
    update_our_and_their_balance(sc, channels_data)
    print("Our balance = " + str(sc.our_balance))
    print("Their balance = " + str(sc.their_balance))

    if sc.is_stable_receiver:
        adjustedBalance = sc.our_balance - sc.native_amount_msat
    else:
        adjustedBalance = sc.their_balance - sc.native_amount_msat

    sc.stable_receiver_dollar_amount = calculate_stable_receiver_dollar_amount(sc, adjustedBalance, expected_msats)
    formatted_time = datetime.utcnow().strftime("%H:%M %d %b %Y")
    print(formatted_time)

    sc.payment_made = False
    amount_too_small = False

    # Scenario 1 - Difference too small to worry about (under $0.01) = do nothing
    if abs(sc.expected_dollar_amount - float(sc.stable_receiver_dollar_amount)) < 0.01:
        print("Scenario 1 - Difference too small to worry about (under $0.01)")
        amount_too_small = True
        sc.payment_made = False

    if not amount_too_small:
        current_stable_receiver_balance = sc.our_balance if sc.is_stable_receiver else sc.their_balance
        msat_difference_from_expected = round(abs(int(expected_msats) - int(current_stable_receiver_balance)))

        # Scenario 2 - Node is stableReceiver and expects to get paid = wait 30 seconds; check on payment
        if sc.stable_receiver_dollar_amount < sc.expected_dollar_amount and sc.is_stable_receiver:
            print("Scenario 2 - Node is stableReceiver and expects to get paid ")
            time.sleep(30)
            channels_data = get_channel_info(sc)
            update_our_and_their_balance(sc, channels_data)
            new_our_stable_balance_msat = sc.our_balance - sc.native_amount_msat
            new_stable_receiver_dollar_amount = calculate_stable_receiver_dollar_amount(sc, new_our_stable_balance_msat, expected_msats)
            if sc.expected_dollar_amount - float(new_stable_receiver_dollar_amount) < 0.01:
                sc.payment_made = True
            else:
                sc.risk_score += 1

        # Scenario 3 - Node is stableProvider and needs to pay = keysend and exit
        elif not sc.is_stable_receiver and sc.stable_receiver_dollar_amount < sc.expected_dollar_amount:

            print("Scenario 3 - Node is stableProvider and needs to pay")
            
            print(str(msat_difference_from_expected))
            print(sc.counterparty)
            print(sc)

            response = keysend_payment(sc, msat_difference_from_expected)
            
            if response.status_code == 200:
                print("Keysend successful:", response.json())
            else:
                print("Failed to send keysend:", response.status_code, response.text)
            sc.payment_made = True

        # Scenario 4 - Node is stableReceiver and needs to pay = keysend and exit
        elif sc.is_stable_receiver and sc.stable_receiver_dollar_amount > sc.expected_dollar_amount:
            print("Scenario 4 - Node is stableReceiver and needs to pay")
            response = keysend_payment(sc, msat_difference_from_expected)
            if response.status_code == 200:
                print("Keysend successful:", response.json())
            else:
                print("Failed to send keysend:", response.status_code, response.text)
            sc.payment_made = True

        # Scenario 5 - Node is stableProvider and expects to get paid = wait 30 seconds; check on payment
        elif not sc.is_stable_receiver and sc.stable_receiver_dollar_amount > sc.expected_dollar_amount:
            print("Scenario 5 - Node is stableProvider and expects to get paid")
            time.sleep(30)
            channels_data = get_channel_info(sc)
            update_our_and_their_balance(sc, channels_data)
            new_their_stable_balance_msat = sc.their_balance - sc.native_amount_msat
            new_stable_receiver_dollar_amount = calculate_stable_receiver_dollar_amount(sc, new_their_stable_balance_msat, expected_msats)
            if sc.expected_dollar_amount - float(new_stable_receiver_dollar_amount) > 0.01:
                sc.payment_made = True
            else:
                sc.risk_score += 1

    json_line = f'{{"formatted_time": "{formatted_time}", "estimated_price": {estimated_price}, "expected_dollar_amount": {sc.expected_dollar_amount}, "stable_receiver_dollar_amount": {sc.stable_receiver_dollar_amount}, "payment_made": {sc.payment_made}, "risk_score": {sc.risk_score}}},\n'
    file_path = '/Users/t/Desktop/stable-channels/stablelog1.json' if sc.is_stable_receiver else '/Users/t/Desktop/stable-channels/stablelog2.json'
    with open(file_path, 'a') as file:
        file.write(json_line)

def main():
    parser = argparse.ArgumentParser(description='LND Script Arguments')
    parser.add_argument('--lnd-server-url', type=str, required=True, help='LND server address')
    parser.add_argument('--macaroon-path', type=str, required=True, help='Hex-encoded macaroon for authentication')
    parser.add_argument('--tls-cert-path', type=str, required=True, help='TLS cert path for server authentication')
    parser.add_argument('--expected-dollar-amount', type=float, required=True, help='Expected dollar amount')
    parser.add_argument('--channel-id', type=str, required=True, help='LND channel ID')
    parser.add_argument('--native-amount-sat', type=float, required=True, help='Native amount in msat')
    parser.add_argument('--is-stable-receiver', type=lambda x: (str(x).lower() == 'true'), required=True, help='Is stable receiver flag')
    parser.add_argument('--counterparty', type=str, required=True, help='LN Node ID of counterparty')

    args = parser.parse_args()

    print(args.lnd_server_url)

    sc = StableChannel(
        channel_id=args.channel_id, 
        native_amount_msat=int(args.native_amount_sat * 1000),
        expected_dollar_amount=args.expected_dollar_amount,
        is_stable_receiver=args.is_stable_receiver,
        counterparty=args.counterparty,
        our_balance=0,
        their_balance=0,
        risk_score=0,
        stable_receiver_dollar_amount=0,
        stable_provider_dollar_amount=0,
        timestamp=0,
        formatted_datetime='',
        payment_made=False,
        lnd_server_url=args.lnd_server_url,
        macaroon_hex=codecs.encode(open(args.macaroon_path, 'rb').read(), 'hex'),
        tls_cert_path = args.tls_cert_path
    )

    print("Initializating a Stable Channel with these details:")
    print(sc)

    thread = threading.Thread(target=start_scheduler, args=(sc,))
    thread.start()
    thread.join()

if __name__ == "__main__":
    main()


# curl --cacert /Users/t/.polar/networks/8/volumes/lnd/alice/tls.cert \
#      --header "Grpc-Metadata-macaroon: 0201036c6e6402f801030a103def9a17d1aa8476cb50a975bb3ef1ee1201301a160a0761646472657373120472656164120577726974651a130a04696e666f120472656164120577726974651a170a08696e766f69636573120472656164120577726974651a210a086d616361726f6f6e120867656e6572617465120472656164120577726974651a160a076d657373616765120472656164120577726974651a170a086f6666636861696e120472656164120577726974651a160a076f6e636861696e120472656164120577726974651a140a057065657273120472656164120577726974651a180a067369676e6572120867656e657261746512047265616400000620f0e59d2963cb3246e9fd8d835ea47e056bf4eb80002ae98204b62bcb5ebf3809" \
#      https://127.0.0.1:8081/v1/balance/channels 

# Alice local startup LND as Stable Receiver
# python3 lnd.py --tls-cert-path=/Users/t/.polar/networks/18/volumes/lnd/alice/tls.cert --expected-dollar-amount=100 --channel-id=125344325632000 --is-stable-receiver=True --counterparty=031786135987ebd4c08999a4cbbae38f67f41828879d191a5c56092e408e1ce9c4 --macaroon-path=/Users/t/.polar/networks/18/volumes/lnd/alice/data/chain/bitcoin/regtest/admin.macaroon --native-amount-sat=0 --lnd-server-url=https://127.0.0.1:8081


# Bob local startup LND as Stable Provider
# python3 lnd.py --tls-cert-path=/Users/t/.polar/networks/18/volumes/lnd/bob/tls.cert --expected-dollar-amount=100 --channel-id=125344325632000 --is-stable-receiver=False --counterparty=030c66a66743e9f9802780c16cc0d97151c6dae61df450dbca276478dc7d0c931d --macaroon-path=/Users/t/.polar/networks/18/volumes/lnd/bob/data/chain/bitcoin/regtest/admin.macaroon --native-amount-sat=0 --lnd-server-url=https://127.0.0.1:8082
