import sys      # for taking in command line arguments
import requests # for http requests
import time     # for timestamps
import json     # for handling json

# populate in-memory variables from command line
channel_id = sys.argv[1]
our_macaroon = sys.argv[2]
is_stable_receiver = sys.argv[3]
expected_dollar_amount = float(sys.argv[4])

# initialize other static variables
headers = {'macaroon':our_macaroon}
base_URL = 'http://127.0.0.1:8183'
endpoint_get_info = '/v1/getinfo'
endpoint_list_channels = '/v1/channel/listchannels'
endpoint_gen_invoice='/v1/invoice/genInvoice'
endpoint_pay_invoice='/v1/pay'
endpoint_keysend='/v1/pay/keysend'

# changeable variables
deliquency_meter = 0

def get_their_node_id():
    response=requests.get(base_URL + endpoint_list_channels, headers=headers)
    json_response=json.loads(str(response.text))
    for obj in json_response:
        if obj.get('channel_id') == channel_id:
            return obj.get('id')

def get_our_balance():
    response=requests.get(base_URL + endpoint_list_channels, headers=headers)
    return json.loads(str(response.text))[0].get('msatoshi_to_us')

def get_their_balance():
    response=requests.get(base_URL + endpoint_list_channels, headers=headers)
    return json.loads(str(response.text))[0].get('msatoshi_to_them')
   
def check_delinquency(peer_id, is_offline, owes_money):
    if stablePartner.is_offline:
        deliquency_count += 1
    if stablePartner.owes_money:
        deliquency_count += 1
    if stablePartner.deliquency_count > 4:
        # too delinquent
        print("Alert for peerID: " + str(peer_id))
    return deliquency_count

# Binance
def get_price_binance():
    response = requests.get("https://api.binance.us/api/v3/avgPrice?symbol=BTCUSDT")
    return float(json.loads(str(response.text)).get("price"))
  
# # Kraken
# def get_price_kraken():

# will only pay to the stable partner ('their_node_id')
def keysend(amount):
    headers = {   
        'Content-Type': 'application/json',
        'macaroon': our_macaroon
    }
    data = { 
        'pubkey': their_node_id,
        'amount':amount
    }
    print(data)
    response = requests.post(base_URL + endpoint_keysend, headers=headers, data=json.dumps(data))
    print(response.text)

their_node_id = get_their_node_id();

is_in_stable_mode = True

while is_in_stable_mode:
    # get respective balances
    our_balance = get_our_balance();
    their_balance =  get_their_balance();

    # get price and actual dollar amount
    price = get_price_binance();

    # need to modify to handle both sides
    actual_dollar_amount = (our_balance / 100000000000) * price
    print(actual_dollar_amount)

    if actual_dollar_amount < expected_dollar_amount and not(is_stable_receiver):
        print("Stable Receiver needs to get paid.")
        need_to_pay_amount = (expected_dollar_amount - actual_dollar_amount) * 100000000000
        print("need to pay amt = " + need_to_pay_amount)
        keysend(need_to_pay_amount)
        break

    elif actual_dollar_amount == expected_dollar_amount:
        print("Juuust right")

    elif actual_dollar_amount > expected_dollar_amount and is_stable_receiver:
        print("Stable provider needs to get paid .")
        need_to_pay_amount = round((actual_dollar_amount - expected_dollar_amount) * 100000000000)
        print("need to pay amt = " + str(need_to_pay_amount))
        keysend(need_to_pay_amount)
        break


        # keysend();



# our_macaroon='AgELYy1saWdodG5pbmcCPlNhdCBOb3YgMDUgMjAyMiAwNTowMzoxMiBHTVQrMDAwMCAoQ29vcmRpbmF0ZWQgVW5pdmVyc2FsIFRpbWUpAAAGIBcnf+0eDYq75V0fKEN42ulqrTHPRQAJ0JY6MBTaLAV3'
# self_is_stable_receiver = True



# def invoice_them(amount):
#     global invoice

#     # get timestamp
#     timestamp = int(time.time())

#     headers = {   
#         'Content-Type': 'application/json',
#         'macaroon': our_macaroon
#     }

#     data = { 
#         'amount': amount,
#         'label': timestamp,
#         'description': 'more info here would be perfect'
#     }

#     response = requests.post(base_URL + endpoint_gen_invoice, headers=headers, data=json.dumps(data))
#     json_reponse = json.loads(response.text)
#     invoice = json_reponse.get('bolt11')
    
# def pay_invoice(invoice):
#     data = { 
#         'invoice': invoice,
#     }
#     headers = {'macaroon': bob_macaroon}
#     print(data)
#     response = requests.post(bob_URL + endpoint_pay_invoice, headers=headers, data=data)
#     print(response.text)

# def get_our_node_id():
#     response = requests.get(base_URL + endpoint_get_info, headers=headers)
#     our_node_id=json.loads(str(response.text)).get('id')
