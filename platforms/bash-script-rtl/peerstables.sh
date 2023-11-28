#!/bin/bash

aliceMacaroon=AgELYy1saWdodG5pbmcCPlNhdCBOb3YgMDUgMjAyMiAwNTowMzoxMiBHTVQrMDAwMCAoQ29vcmRpbmF0ZWQgVW5pdmVyc2FsIFRpbWUpAAAGIBcnf+0eDYq75V0fKEN42ulqrTHPRQAJ0JY6MBTaLAV3

bobMacaroon=AgELYy1saWdodG5pbmcCPlNhdCBOb3YgMDUgMjAyMiAwNTowMzoxMyBHTVQrMDAwMCAoQ29vcmRpbmF0ZWQgVW5pdmVyc2FsIFRpbWUpAAAGIJIngPnzHfCi2+Cv8v0Iz6o6xYDseOA6G41Op+I4+wEg

echo "
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣠⣤⣴⣶⣶⣿⣿⣿⣿⣿⣿⣿⣿⣶⣶⣶⣤⣄⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣠⣴⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣦⣄⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣤⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣤⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣠⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⣠⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣄⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⣠⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠉⠛⠛⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣄⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⣴⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⠀⠀⢰⣿⣿⠇⠀⠉⠉⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣦⠀⠀⠀⠀⠀
⠀⠀⠀⢀⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠉⠉⠛⠛⠿⠿⡏⠀⠀⠀⣾⣿⡿⠀⠀⠀⣸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⡀⠀⠀⠀
⠀⠀⢀⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠙⠛⠃⠀⠀⢀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⡀⠀⠀
⠀⠀⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⣆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠛⠻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⠀⠀
⠀⣸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀⠀⠀⠀⠀⢠⣶⣦⣤⣀⡀⠀⠀⠀⠀⠀⠀⠙⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣇⠀
⢀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⠀⠀⠀⠀⠀⣼⣿⣿⣿⣿⣿⣷⡄⠀⠀⠀⠀⠀⠈⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡄
⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀⠀⠀⠀⠀⢠⣿⣿⣿⣿⣿⣿⣿⡿⠀⠀⠀⠀⠀⠀⣸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇
⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⠀⠀⠀⠀⠀⠘⠿⠿⢿⣿⣿⡿⠟⠁⠀⠀⠀⠀⠀⢀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣠⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⠀⠀⠀⠀⠀⣴⣶⣤⣤⣀⡀⠀⠀⠀⠀⠀⠀⠐⠿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀⠀⠀⠀⠀⢠⣿⣿⣿⣿⣿⣿⣷⣦⡀⠀⠀⠀⠀⠀⠘⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠿⠿⢿⡿⠁⠀⠀⠀⠀⠀⣼⣿⣿⣿⣿⣿⣿⣿⣿⣧⠀⠀⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇
⠈⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠏⠀⠀⠀⠀⠀⠀⠀⠀⠀⠠⣿⣿⣿⣿⣿⣿⣿⣿⣿⠇⠀⠀⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠃
⠀⢹⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣯⣄⣀⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠉⠉⠉⠉⠀⠀⠀⠀⠀⠀⠀⢠⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀
⠀⠀⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⠀⠀⠀⢀⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣠⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠀⠀
⠀⠀⠈⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⠀⠀⠀⣾⣿⣿⠀⠀⠀⢠⣤⣄⣀⣀⣀⣀⣤⣴⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠁⠀⠀
⠀⠀⠀⠈⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⠀⠀⢰⣿⣿⡇⠀⠀⢀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠁⠀⠀⠀
⠀⠀⠀⠀⠀⠻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣶⣦⣾⣿⣿⡀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠟⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠙⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠋⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠙⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠋⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠙⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠋⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠛⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠛⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠙⠻⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠟⠋⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠙⠛⠻⠿⠿⢿⣿⣿⣿⣿⣿⣿⡿⠿⠿⠟⠛⠋⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀                                                        
"
function getAliceID() {
	aliceID=$(curl -s -H "macaroon:$aliceMacaroon" http://127.0.0.1:8181/v1/getinfo | jq -r '.id');
}


function getBobID() {
	bobID=$(curl -s -H "macaroon:$bobMacaroon" http://127.0.0.1:8182/v1/getinfo | jq -r '.id');
}

function getAliceBalance() {
	aliceBalance=$(curl -s -H "macaroon:$aliceMacaroon" http://127.0.0.1:8181/v1/channel/listchannels | jq '.[0].msatoshi_to_us')
}

function getBobBalance() {
	bobBalance=$(curl -s -H "macaroon:$bobMacaroon" http://127.0.0.1:8182/v1/channel/listchannels | jq '.[0].msatoshi_to_us')
}

function getAliceInvoice() {
	# echo "in Alice's invoice function"
	timestampLabel=$(date +%s)
	amt=$1
	# echo $amt
	aliceInvoice=$(curl -s -X POST http://127.0.0.1:8181/v1/invoice/genInvoice -H "macaroon:$aliceMacaroon" -H "Content-Type: application/json" -d '{"amount":"'$amt'","label":"'$timestampLabel'","description":"booyakasha"}'  | jq -r '.bolt11');
	# echo $aliceInvoice

}

function getBobInvoice() {
	# echo "in Bob's invoice function"
	timestampLabel=$(date +%s)
	# echo $timestampLabel
	amt=$1
	# echo $amt
	bobInvoice=$(curl -s -X POST http://127.0.0.1:8182/v1/invoice/genInvoice -H "macaroon:$bobMacaroon" -H "Content-Type: application/json" -d '{"amount":"'$amt'","label":"'$timestampLabel'","description":"booyakasha"}' | jq -r '.bolt11');
}

function alicePays() {
	# echo "in Alice's pay function"
	bobInvoice=$1
	# echo "bobInvoice"
	# echo $bobInvoice
	curl -s -X POST http://127.0.0.1:8181/v1/pay -H "macaroon:$aliceMacaroon" -H "Content-Type: application/json" -d '{"invoice":"'$bobInvoice'"}' | jq
}

function bobPays() {
	# echo "in Bob's pay function"
	aliceInvoice=$1
	# echo "aliceInvoice"
	# echo $aliceInvoice
	curl -s -X POST http://127.0.0.1:8182/v1/pay -H "macaroon:$bobMacaroon" -H "Content-Type: application/json" -d '{"invoice":"'$aliceInvoice'"}' | jq 
}


function getPrice() {
	curl -s -A "Mozilla/5.0 (X11; Linux x86_64; rv:60.0) Gecko/20100101 Firefox/81.0" https://river.com/bitcoin-price > scratch.txt;
	tr -d '' < scratch.txt > scratch2.txt
	# interim=$(grep -E -o "p class=\"js-nav-price c-home__bitcoin-price--price\".{0,12}" scratch2.txt);
	price=$(sed '128!d' scratch2.txt); 
	# price=${interim:53};
	echo "price"
	echo $price

	now=$(date)
    
}

getAliceID
getBobID
sleep 2
printf "<h1>Welcome to peerstables.org: peer-to-peer stable channels</h1>" >> /opt/homebrew/var/www/index.html
echo "<br>" >> /opt/homebrew/var/www/index.html
printf "<body>starting peer stable server ... </body>" >> /opt/homebrew/var/www/index.html
sleep 2
printf "<body>&emsp;started ...  " >> /opt/homebrew/var/www/index.html
echo "<br>" >> /opt/homebrew/var/www/index.html
sleep 2
printf "<body>generating new at-will peer stable agreement for \$50...  </body>" >> /opt/homebrew/var/www/index.html
echo "<br>" >> /opt/homebrew/var/www/index.html
sleep 3
printf "<body>two counterparties:</body>" >> /opt/homebrew/var/www/index.html
echo "<br>" >> /opt/homebrew/var/www/index.html
printf "<body>&emsp;stableReceiver is Alice = $aliceID</body>" >> /opt/homebrew/var/www/index.html
echo "<br>" >> /opt/homebrew/var/www/index.html
printf "<body>&emsp;stableProvider is Bob = $bobID</body>" >> /opt/homebrew/var/www/index.html
echo "<br>" >> /opt/homebrew/var/www/index.html
sleep 3

printf "<body>peers connected ...</body>" >> /opt/homebrew/var/www/index.html
sleep 2
printf "<body>&emsp;channel established ...</body>" >> /opt/homebrew/var/www/index.html
echo "<br>" >> /opt/homebrew/var/www/index.html
sleep 2


printf "<body>&emsp;entering stable mode, buckle up!</body>" >> /opt/homebrew/var/www/index.html
echo "<br><br>" >> /opt/homebrew/var/www/index.html

for i in {1..500}

	do
		now=$(date)

		getAliceBalance
		getBobBalance
		getPrice
		# echo "stableReceiver Alice balance = $aliceBalance "
		# echo "stableProvider Bob balance = $bobBalance "
        echo "Round $i &emsp;" >> /opt/homebrew/var/www/index.html
		echo "<body>current bitcoin price is $price</body>" >> /opt/homebrew/var/www/index.html
		echo "<br>" >> /opt/homebrew/var/www/index.html

		sleep 3

		price2=$(echo $price | sed 's/,//')
		price2="${price2:1}"
		echo "price2"
		echo $price2
		aliceBalanceBtc=$(echo "scale=8;$aliceBalance/100000000000" | bc)

		# echo "Alice stableReceiver bitcoin balance: $aliceBalanceBtc"
		

		expectedDollarAmount=50.00
		echo "<body>&emsp;expectedDollarAmount = $expectedDollarAmount</body>" >> /opt/homebrew/var/www/index.html
		echo "<br>" >> /opt/homebrew/var/www/index.html
		sleep 2
		actualDollarAmount=$(echo "scale=2;$aliceBalanceBtc*$price2" | bc)

		echo "<body>&emsp;actualDollarAmount = $actualDollarAmount </body>" >> /opt/homebrew/var/www/index.html
		echo "<br>" >> /opt/homebrew/var/www/index.html
		sleep 2

		if (( $(echo "$actualDollarAmount < $expectedDollarAmount" | bc -l) )); then
		  echo "<body>&emsp;actualDollarAmount less than expectedDollarAmount:</body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  echo "<body>&emsp;Bob needs to pay Alice.</body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  
		  needToPayDollarAmount=$(echo "$expectedDollarAmount - $actualDollarAmount" | bc -l)
		  needToPayBitcoinAmount=$(echo "$needToPayDollarAmount / $price2" | bc -l) 
		  echo "<body>&emsp;Bob needs to pay this much in dollars: $needToPayDollarAmount</body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  echo "<body>&emsp;Bob needs to pay this much in bitcoin: $needToPayBitcoinAmount</body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  # echo $needToPayBitcoinAmount
		  needToPayBitcoinAmount="${needToPayBitcoinAmount:1}"
		  needToPayBitcoinAmount="${needToPayBitcoinAmount%?????????}"
		  getAliceInvoice $needToPayBitcoinAmount
		  sleep 5
		  # Pay alice what is needed
		  
		  echo "<body>&emsp;Bob is paying now.</body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  
		  
		  bobPays $aliceInvoice
		fi

		if (( $(echo "$actualDollarAmount == $expectedDollarAmount" | bc -l) )); then
		  echo "<body>&emsp;actualDollarAmount equals expectedDollarAmount: <body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  echo "<body>&emsp;No payment needed.</body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html

		fi

		if (( $(echo "$actualDollarAmount > $expectedDollarAmount" | bc -l) )); then	
		  echo "<body>&emsp;actualDollarAmount more than expectedDollarAmount: </body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  echo "<body>&emsp;Alice needs to pay Bob. </body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  needToPayDollarAmount=$(echo "$actualDollarAmount - $expectedDollarAmount" | bc -l)
		  echo "<body>&emsp;Alice needs to pay this much in dollars: $needToPayDollarAmount </body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  needToPayBitcoinAmount=$(echo "$needToPayDollarAmount / $price2" | bc -l) 
		  echo "<body>&emsp;lice needs to pay this much in bitcoin: $needToPayBitcoinAmount</body>" >> /opt/homebrew/var/www/index.html
		  echo "<br>" >> /opt/homebrew/var/www/index.html
		  needToPayBitcoinAmount="${needToPayBitcoinAmount:1}"
		  needToPayBitcoinAmount="${needToPayBitcoinAmount%?????????}"

		  getBobInvoice $needToPayBitcoinAmount

		  # echo "bobInvoice"
		  # echo $bobInvoice

		  sleep 5
		  # Pay bob what is needed
		  
		  echo "<body>&emsp;Alice is paying now.:</body>" >> /opt/homebrew/var/www/index.html
		  
		  
		  alicePays $bobInvoice
		fi

        
		
        echo "<body>&emsp;payment complete!</body>" >> /opt/homebrew/var/www/index.html
        echo "<br>" >> /opt/homebrew/var/www/index.html
        echo "<br>" >> /opt/homebrew/var/www/index.html
		printf "<body>waiting 30 seconds until next price query ...</body>" >> /opt/homebrew/var/www/index.html
		sleep 10
		echo "<br>..." >> /opt/homebrew/var/www/index.html
		sleep 10
		echo "<br>..." >> /opt/homebrew/var/www/index.html
        sleep 10
		echo "<br>" >> /opt/homebrew/var/www/index.html
    done

