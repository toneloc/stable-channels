//
//  StableChannelsApp.swift
//  StableChannels
//
//  Created by tony on 10/4/23.
//

import SwiftUI

@main
struct StableChannelsApp: App {
    let persistenceController = PersistenceController.shared

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(\.managedObjectContext, persistenceController.container.viewContext)
        }
    }
}
