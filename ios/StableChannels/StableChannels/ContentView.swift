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
                Spacer()  // Push the button to the far right
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
            
            Spacer().frame(height: (UIScreen.main.bounds.height * 0.25) - 120)
            
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
                    self.balance -= 1.00
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
