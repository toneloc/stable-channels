package com.stablechannels.app.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccessTime
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.stablechannels.app.AppState
import com.stablechannels.app.ui.history.HistoryScreen
import com.stablechannels.app.ui.home.HomeScreen
import com.stablechannels.app.ui.settings.SettingsNavHost

enum class Tab(val label: String, val icon: ImageVector) {
    HOME("Home", Icons.Default.Home),
    HISTORY("History", Icons.Default.AccessTime),
    SETTINGS("Settings", Icons.Default.Settings)
}

@Composable
fun MainTabView(appState: AppState) {
    var selectedTab by remember { mutableStateOf(Tab.HOME) }
    var showBottomBar by remember { mutableStateOf(true) }
    val systemBarsPadding = WindowInsets.systemBars.asPaddingValues()

    LaunchedEffect(selectedTab) {
        if (selectedTab != Tab.SETTINGS) {
            showBottomBar = true
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .padding(systemBarsPadding)
    ) {
        when (selectedTab) {
            Tab.HOME -> HomeScreen(appState)
            Tab.HISTORY -> HistoryScreen(appState)
            Tab.SETTINGS -> SettingsNavHost(appState, onShowBottomBar = { showBottomBar = it })
        }
        if (showBottomBar) {
            ModernBottomNavBar(
                selectedTab = selectedTab,
                onTabSelected = { selectedTab = it },
                modifier = Modifier.align(Alignment.BottomCenter)
            )
        }
    }
}

@Composable
fun ModernBottomNavBar(
    selectedTab: Tab,
    onTabSelected: (Tab) -> Unit,
    modifier: Modifier = Modifier
) {
    val isDark = isSystemInDarkTheme()
    Box(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 92.dp, vertical = 12.dp)
    ) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .then(
                    if (isDark) Modifier.border(
                        width = 0.5.dp,
                        color = MaterialTheme.colorScheme.outlineVariant,
                        shape = RoundedCornerShape(20.dp)
                    ) else Modifier
                ),
            shape = RoundedCornerShape(20.dp),
            shadowElevation = if (isDark) 0.dp else 12.dp,
            color = MaterialTheme.colorScheme.surface
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 4.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.SpaceEvenly
            ) {
                Tab.entries.forEach { tab ->
                    val isSelected = selectedTab == tab
                    val animatedColor by animateColorAsState(
                        targetValue = if (isSelected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        animationSpec = tween(200),
                        label = "color"
                    )

                    Box(
                        contentAlignment = Alignment.Center,
                        modifier = Modifier
                            .size(44.dp)
                            .clip(RoundedCornerShape(12.dp))
                            .clickable(
                                interactionSource = remember { MutableInteractionSource() },
                                indication = null
                            ) { onTabSelected(tab) }
                            .background(
                                color = if (isSelected) MaterialTheme.colorScheme.primary.copy(alpha = 0.12f) else Color.Transparent,
                                shape = RoundedCornerShape(12.dp)
                            )
                    ) {
                        Icon(
                            tab.icon,
                            contentDescription = tab.label,
                            tint = animatedColor,
                            modifier = Modifier.size(22.dp)
                        )
                    }
                }
            }
        }
    }
}
