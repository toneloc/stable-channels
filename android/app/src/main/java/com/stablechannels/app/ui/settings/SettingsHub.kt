package com.stablechannels.app.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.navigation.NavController
import com.stablechannels.app.AppState

@Composable
fun SettingsHub(appState: AppState, navController: NavController) {
    val onchainSats by appState.onchainBalanceSats.collectAsState()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
    ) {
        Text(
            text = "Settings",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 16.dp)
        )
        Spacer(Modifier.height(12.dp))

        // Wallet section
        SettingsSectionHeader(title = "Wallet", color = Color(0xFF10B981))
            SettingsNavLink(
                icon = Icons.Default.AccountBalance,
                iconBackground = Color(0xFF10B981),
                label = "Stable Position",
                onClick = { navController.navigate(SettingsRoute.StablePosition.route) }
            )
            SettingsNavLink(
                icon = Icons.Default.Hub,
                iconBackground = Color(0xFF10B981),
                label = "Channel",
                onClick = { navController.navigate(SettingsRoute.Channel.route) }
            )
            SettingsNavLink(
                icon = Icons.Default.Key,
                iconBackground = Color(0xFF10B981),
                label = "Backup",
                onClick = { navController.navigate(SettingsRoute.Backup.route) }
            )
            if (onchainSats > 0) {
                SettingsNavLink(
                    icon = Icons.Default.Send,
                    iconBackground = Color(0xFF10B981),
                    label = "Send On-Chain",
                    onClick = { navController.navigate(SettingsRoute.OnChainSend.route) }
                )
            }

            Spacer(Modifier.height(16.dp))

            // Preferences section
            SettingsSectionHeader(title = "Preferences", color = Color(0xFF8B5CF6))
            SettingsNavLink(
                icon = Icons.Default.Palette,
                iconBackground = Color(0xFF8B5CF6),
                label = "Appearance",
                onClick = { navController.navigate(SettingsRoute.Appearance.route) }
            )
            SettingsNavLink(
                icon = Icons.Default.Notifications,
                iconBackground = Color(0xFF8B5CF6),
                label = "Notifications",
                onClick = { navController.navigate(SettingsRoute.Notifications.route) }
            )

            Spacer(Modifier.height(16.dp))

            // Node & Network section
            SettingsSectionHeader(title = "Node & Network", color = Color(0xFF3B82F6))
            SettingsNavLink(
                icon = Icons.Default.Memory,
                iconBackground = Color(0xFF3B82F6),
                label = "Node",
                onClick = { navController.navigate(SettingsRoute.Node.route) }
            )
            SettingsNavLink(
                icon = Icons.Default.Cloud,
                iconBackground = Color(0xFF3B82F6),
                label = "Push Connectivity",
                onClick = { navController.navigate(SettingsRoute.PushConnectivity.route) }
            )

            Spacer(Modifier.height(16.dp))

            // Privacy & Security section
            SettingsSectionHeader(title = "Privacy & Security", color = Color(0xFF6366F1))
            SettingsNavLink(
                icon = Icons.Default.Lock,
                iconBackground = Color(0xFF6366F1),
                label = "App Access",
                onClick = { navController.navigate(SettingsRoute.AppAccess.route) }
            )

            Spacer(Modifier.height(16.dp))

            // About section
            SettingsSectionHeader(title = "About", color = Color(0xFF6B7280))
            SettingsNavLink(
                icon = Icons.Default.Info,
                iconBackground = Color(0xFF6B7280),
                label = "About",
                onClick = { navController.navigate(SettingsRoute.About.route) }
            )

            Spacer(Modifier.height(32.dp))
        }
}

@Composable
private fun SettingsSectionHeader(title: String, color: Color) {
    Text(
        text = title,
        style = MaterialTheme.typography.titleMedium,
        fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
        color = color,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 8.dp)
    )
}

@Composable
private fun SettingsNavLink(
    icon: ImageVector,
    iconBackground: Color,
    label: String,
    onClick: () -> Unit
) {
    ListItem(
        headlineContent = { Text(label) },
        leadingContent = {
            Box(
                modifier = Modifier
                    .size(32.dp)
                    .background(
                        color = iconBackground.copy(alpha = 0.15f),
                        shape = RoundedCornerShape(8.dp)
                    ),
                contentAlignment = Alignment.Center
            ) {
                Icon(
                    imageVector = icon,
                    contentDescription = label,
                    tint = iconBackground,
                    modifier = Modifier.size(18.dp)
                )
            }
        },
        trailingContent = {
            Icon(
                imageVector = Icons.Default.ChevronRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant
            )
        },
        modifier = Modifier.clickable(onClick = onClick)
    )
}
