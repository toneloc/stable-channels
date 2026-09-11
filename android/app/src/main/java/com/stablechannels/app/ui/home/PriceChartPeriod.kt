package com.stablechannels.app.ui.home

import java.util.Calendar

/**
 * Time periods available for price chart display.
 */
enum class ChartPeriod(val label: String, val days: Int, val usesHourly: Boolean) {
    DAY_1("1D", 1, true),
    WEEK_1("1W", 7, true),
    MONTH_1("1M", 30, true),
    MONTH_3("3M", 90, false),
    MONTH_6("6M", 180, false),
    YTD("YTD", -1, false),  // computed dynamically
    YEAR_1("1Y", 365, false),
    YEAR_2("2Y", 730, false),
    YEAR_5("5Y", 1825, false),
    YEAR_10("10Y", 3650, false),
    ALL("ALL", 99999, false);

    fun effectiveDays(): Int {
        if (this == YTD) {
            val cal = Calendar.getInstance()
            return cal.get(Calendar.DAY_OF_YEAR)
        }
        return days
    }
}
