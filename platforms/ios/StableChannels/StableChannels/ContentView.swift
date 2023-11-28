import SwiftUI
import Combine

struct ContentView: View {
    @State private var balance: Double = 0.00
    @State private var bitcoinPrice: Double = 0.00
    @State private var lastUpdated: Date?
    @State private var cancellable: AnyCancellable?
    @State private var showMenu: Bool = false

    var body: some View {
        VStack(spacing: 20) {
            HStack {
                Image("StableChannelsIcon")  // <-- Add this line here
                           .resizable()
                           .scaledToFit()
                           .frame(width: 45, height: 45)  // Adjust size as needed
                           .padding(.trailing, 20)  // Optional padding to separate the icon from other elements

                Spacer()
                
                Button(action: {
                    showMenu.toggle()
                }) {
                    Image(systemName: "line.horizontal.3")
                        .font(.title)
                        .padding()
                }
                .actionSheet(isPresented: $showMenu) {
                    ActionSheet(title: Text("Menu"), buttons: [
                        .default(Text("Close Stable Channel")),
                        .default(Text("Review on-chain details")),
                        .default(Text("Show recovery phrase")),
                        .cancel()
                    ])
                }
            }

            Text("Stable Receiver")
                       .font(.largeTitle)
                       .padding(.top, 20)
            
            Spacer().frame(height: (UIScreen.main.bounds.height * 0.25) - 170)
            
            Text("USD balance")
                .font(.title2)
                
            Text(String(format: "$%.2f", balance))
                .font(.largeTitle)
            
            VStack(spacing: 5) {
                Text("Bitcoin Price: \(String(format: "$%.2f", bitcoinPrice))")
                    .font(.caption)
                    .foregroundColor(.gray)
                            
                if let lastUpdated = lastUpdated {
                    Text("Last Updated: \(formattedDate(from: lastUpdated))")
                        .font(.caption)
                        .foregroundColor(.gray)
                }
            }

            HStack(spacing: 20) {
                Button(action: {
                    self.sendPayment()
                }) {
                    HStack {
                        Image(systemName: "arrow.up.right.square.fill")
                        Text("Send")
                    }
                    .padding()
                    .frame(width: 150, height: 50)
                    .background(LinearGradient(gradient: Gradient(colors: [Color.blue, Color.blue.opacity(0.7)]), startPoint: .top, endPoint: .bottom))
                    .foregroundColor(.white)
                    .cornerRadius(25)
                    .shadow(color: Color.blue.opacity(0.4), radius: 10, x: 0, y: 10)
                }
                Button(action: {
                    self.balance += 1.00
                }) {
                    HStack {
                        Image(systemName: "arrow.down.left.square.fill")
                        Text("Receive")
                    }
                    .padding()
                    .frame(width: 150, height: 50)
                    .background(LinearGradient(gradient: Gradient(colors: [Color.gray, Color.gray.opacity(0.7)]), startPoint: .top, endPoint: .bottom))
                    .foregroundColor(.white)
                    .cornerRadius(25)
                    .shadow(color: Color.gray.opacity(0.4), radius: 10, x: 0, y: 10)
                }
            }
            
            Spacer()
        }
        .padding()
        .onAppear {
            fetchBalance()
            print("fetching")
            Timer.scheduledTimer(withTimeInterval: 30.0, repeats: true) { _ in
                self.fetchBalance()
            }
        }
        .onDisappear {
            self.cancellable?.cancel()
        }
    }

    func fetchBalance() {
        let url = URL(string: "https://stablechannels.com/balance")!

        cancellable = URLSession.shared.dataTaskPublisher(for: url)
            .map { $0.data }
            .decode(type: BalanceResponse.self, decoder: JSONDecoder())
            .receive(on: DispatchQueue.main)
            .sink(receiveCompletion: { completion in
                switch completion {
                case .finished: break
                case .failure(let error): print("Error: \(error)")
                }
            }, receiveValue: { balanceResponse in
                self.balance = Double(balanceResponse.balance)
                self.bitcoinPrice = Double(balanceResponse.price)
                self.lastUpdated = Date()
            })
    }

    func formattedDate(from date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
    
    func sendPayment() {
        guard let url = URL(string: "https://stablechannels.com/keysend") else {
            print("Invalid URL")
            return
        }

        let paymentData: [String: Any] = [
            "destination": "03affb7a33ebe5d2055c2812af87a63913bd4f6931448e908ffce693544d0d958d",
            "amount_msat": 100
        ]

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.httpBody = try? JSONSerialization.data(withJSONObject: paymentData)
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        let task = URLSession.shared.dataTask(with: request) { data, response, error in
            guard let data = data, error == nil else {
                print("Error in sending payment:", error?.localizedDescription ?? "No data")
                return
            }

            if let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 {
                print("Payment successful. Response:", String(data: data, encoding: .utf8) ?? "")
            } else {
                print("Failed to make payment. Response:", String(data: data, encoding: .utf8) ?? "")
            }
        }

        task.resume()
    }
}

struct BalanceResponse: Decodable {
    let balance: Double
    let price: Double
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
