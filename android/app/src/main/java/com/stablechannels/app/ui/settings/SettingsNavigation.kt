package com.stablechannels.app.ui.settings

import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.stablechannels.app.AppState

sealed class SettingsRoute(val route: String) {
    object Hub : SettingsRoute("settings_hub")
    object StablePosition : SettingsRoute("settings_stable_position")
    object Channel : SettingsRoute("settings_channel")
    object Backup : SettingsRoute("settings_backup")
    object OnChainSend : SettingsRoute("settings_onchain_send")
    object Appearance : SettingsRoute("settings_appearance")
    object Notifications : SettingsRoute("settings_notifications")
    object Node : SettingsRoute("settings_node")
    object PushConnectivity : SettingsRoute("settings_push_connectivity")
    object AppAccess : SettingsRoute("settings_app_access")
    object About : SettingsRoute("settings_about")
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsNavHost(appState: AppState, modifier: Modifier = Modifier) {
    val navController = rememberNavController()

    NavHost(
        navController = navController,
        startDestination = SettingsRoute.Hub.route,
        modifier = modifier
    ) {
        composable(SettingsRoute.Hub.route) {
            SettingsHub(appState = appState, navController = navController)
        }
        composable(SettingsRoute.StablePosition.route) {
            SettingsSubViewScaffold(title = "Stable Position", navController = navController) {
                StablePositionView(appState = appState)
            }
        }
        composable(SettingsRoute.Channel.route) {
            SettingsSubViewScaffold(title = "Channel", navController = navController) {
                ChannelView(appState = appState)
            }
        }
        composable(SettingsRoute.Backup.route) {
            SettingsSubViewScaffold(title = "Backup", navController = navController) {
                BackupView(appState = appState)
            }
        }
        composable(SettingsRoute.OnChainSend.route) {
            SettingsSubViewScaffold(title = "Send On-Chain", navController = navController) {
                OnChainSendSettingsView(appState = appState, onDismiss = { navController.popBackStack() })
            }
        }
        composable(SettingsRoute.Appearance.route) {
            SettingsSubViewScaffold(title = "Appearance", navController = navController) {
                AppearanceView()
            }
        }
        composable(SettingsRoute.Notifications.route) {
            SettingsSubViewScaffold(title = "Notifications", navController = navController) {
                NotificationsView()
            }
        }
        composable(SettingsRoute.Node.route) {
            SettingsSubViewScaffold(title = "Node", navController = navController) {
                NodeView(appState = appState)
            }
        }
        composable(SettingsRoute.PushConnectivity.route) {
            SettingsSubViewScaffold(title = "Push Connectivity", navController = navController) {
                PushConnectivityView()
            }
        }
        composable(SettingsRoute.AppAccess.route) {
            SettingsSubViewScaffold(title = "App Access", navController = navController) {
                AppAccessView()
            }
        }
        composable(SettingsRoute.About.route) {
            SettingsSubViewScaffold(title = "About", navController = navController) {
                AboutView()
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsSubViewScaffold(
    title: String,
    navController: NavHostController,
    content: @Composable () -> Unit
) {
    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = { navController.popBackStack() }) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                }
            )
        }
    ) { padding ->
        Surface(modifier = Modifier.padding(padding)) {
            content()
        }
    }
}
