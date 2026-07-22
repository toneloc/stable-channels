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
import androidx.compose.foundation.border
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.compose.ui.platform.LocalUriHandler
import androidx.navigation.NavController
import com.stablechannels.app.AppState
import com.stablechannels.app.util.Constants

@Composable
fun SettingsHub(appState: AppState, navController: NavController) {
    val onchainSats by appState.onchainBalanceSats.collectAsState()
    val uriHandler = LocalUriHandler.current

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
                .padding(top = 16.dp, bottom = 8.dp)
        )
        
        DisclaimerBanner()
        
        Spacer(Modifier.height(8.dp))

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
                    label = "Send Onchain",
                    onClick = { navController.navigate(SettingsRoute.OnChainSend.route) }
                )
            }

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

            // Privacy & Security section
            SettingsSectionHeader(title = "Privacy & Security", color = Color(0xFF6366F1))
            SettingsNavLink(
                icon = Icons.Default.Lock,
                iconBackground = Color(0xFF6366F1),
                label = "App Access",
                onClick = { navController.navigate(SettingsRoute.AppAccess.route) }
            )
            SettingsNavLink(
                icon = Icons.Default.PrivacyTip,
                iconBackground = Color(0xFF6366F1),
                label = "Privacy Policy",
                onClick = { uriHandler.openUri(Constants.PRIVACY_POLICY_URL) }
            )

            // Support section
            SettingsSectionHeader(title = "Support", color = Color(0xFF10B981))
            SettingsNavLink(
                icon = Icons.Default.MedicalServices,
                iconBackground = Color(0xFF10B981),
                label = "Logs & Diagnostics",
                onClick = { navController.navigate(SettingsRoute.Logs.route) }
            )

            // About section
            SettingsSectionHeader(title = "About", color = Color(0xFF6B7280))
            SettingsNavLink(
                icon = Icons.Default.Info,
                iconBackground = Color(0xFF6B7280),
                label = "About",
                onClick = { navController.navigate(SettingsRoute.About.route) }
            )
            SettingsNavLink(
                icon = Icons.Default.PrivacyTip,
                iconBackground = Color(0xFF6B7280),
                label = "Privacy Policy",
                onClick = { uriHandler.openUri("https://stablechannels.com/privacy.html") }
            )

            Spacer(Modifier.height(100.dp))
        }
}

@Composable
private fun DisclaimerBanner() {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .background(
                color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f),
                shape = RoundedCornerShape(16.dp)
            )
            .border(
                width = 1.dp,
                color = Color(0xFF10B981),
                shape = RoundedCornerShape(16.dp)
            )
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Box(
            modifier = Modifier
                .size(44.dp)
                .background(Color(0xFF10B981), shape = androidx.compose.foundation.shape.CircleShape),
            contentAlignment = Alignment.Center
        ) {
            Icon(
                imageVector = Icons.Default.Security,
                contentDescription = null,
                tint = Color.White,
                modifier = Modifier.size(24.dp)
            )
        }
        Spacer(modifier = Modifier.width(14.dp))
        Column {
            Text(
                text = "Your keys, your coins.",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = androidx.compose.ui.text.font.FontWeight.Bold
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Stable Channels is a self-custodial wallet. You control your private keys. Third parties do not custody, access, or freeze your funds.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun SettingsSectionHeader(title: String, color: Color) {
    Text(
        text = title,
        style = MaterialTheme.typography.titleSmall,
        fontWeight = androidx.compose.ui.text.font.FontWeight.Bold,
        color = color,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 2.dp)
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
        headlineContent = { Text(label, style = MaterialTheme.typography.bodyMedium) },
        leadingContent = {
            Box(
                modifier = Modifier
                    .size(28.dp)
                    .background(
                        color = iconBackground.copy(alpha = 0.15f),
                        shape = RoundedCornerShape(6.dp)
                    ),
                contentAlignment = Alignment.Center
            ) {
                Icon(
                    imageVector = icon,
                    contentDescription = label,
                    tint = iconBackground,
                    modifier = Modifier.size(16.dp)
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
        modifier = Modifier.height(44.dp).clickable(onClick = onClick)
    )
}
