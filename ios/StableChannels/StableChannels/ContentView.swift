import SwiftUI
import Combine

struct ContentView: View {
    @State private var balance: Double = 0.00
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
                
            if let lastUpdated = lastUpdated {
                Text("Last Updated: \(formattedDate(from: lastUpdated))")
                    .font(.caption)
                    .foregroundColor(.gray)
            }

            HStack(spacing: 50) {
                Button(action: {
                    self.balance -= 1.00
                }) {
                    Text("Send")
                        .padding()
                        .background(Color.blue)
                        .foregroundColor(.white)
                        .cornerRadius(10)
                }

                Button(action: {
                    self.balance += 1.00
                }) {
                    Text("Receive")
                        .padding()
                        .background(Color.gray)
                        .foregroundColor(.white)
                        .cornerRadius(10)
                }
            }
            
            Spacer()
        }
        .padding()
        .onAppear {
            fetchBalance()
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
                self.balance = Double(balanceResponse.userId)
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
    let userId: Int
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
