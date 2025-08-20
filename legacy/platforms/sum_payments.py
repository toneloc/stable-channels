import json

with open('stablelog1.json', 'r') as file:
    data = json.load(file)

cumulative_sum = 0
counter =0

for entry in data:
    if entry["payment_made"]:
        difference = abs(entry["stable_receiver_dollar_amount"] - entry["expected_dollar_amount"])
        cumulative_sum += difference
        counter += 1

print(counter)
print(cumulative_sum)

