package com.stablechannels.app.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.history.HistoryScreen
import com.stablechannels.app.ui.home.HomeScreen
import com.stablechannels.app.ui.settings.SettingsScreen

enum class Tab(val label: String, val icon: ImageVector) {
    HOME("Home", Icons.Default.Home),
    HISTORY("History", Icons.Default.History),
    SETTINGS("Settings", Icons.Default.Settings)
}

@Composable
fun MainTabView(appState: AppState) {
    var selectedTab by remember { mutableStateOf(Tab.HOME) }

    Scaffold(
        bottomBar = {
            NavigationBar {
                Tab.entries.forEach { tab ->
                    NavigationBarItem(
                        icon = { Icon(tab.icon, contentDescription = tab.label) },
                        label = { Text(tab.label) },
                        selected = selectedTab == tab,
                        onClick = { selectedTab = tab }
                    )
                }
            }
        }
    ) { padding ->
        when (selectedTab) {
            Tab.HOME -> HomeScreen(appState, Modifier.padding(padding))
            Tab.HISTORY -> HistoryScreen(appState, Modifier.padding(padding))
            Tab.SETTINGS -> SettingsScreen(appState, Modifier.padding(padding))
        }
    }
}
