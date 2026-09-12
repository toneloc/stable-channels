package com.stablechannels.app.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

private const val FULCRUM_X = 511.5f
private const val FULCRUM_Y = 361.0f
private const val LEFT_PIVOT_X = 273.0f
private const val LEFT_PIVOT_Y = 361.0f

private const val MARK_WIDTH = 696.4f
private const val MARK_HEIGHT = 568.0f
private const val MARK_MIN_X = 171.9f
private const val MARK_MIN_Y = 210.0f

val DefaultBrandColor = Color(0xFFF7931A)

@Composable
fun UnifiedBalanceLaunchView(
    modifier: Modifier = Modifier,
    isSyncComplete: Boolean = false,
    size: Dp = 120.dp,
    baseColor: Color = DefaultBrandColor,
    elapsedSeconds: Float? = null,
    previewStage: BalanceScaleKinematics.Stage? = null,
    kinematics: BalanceScaleKinematics = remember { BalanceScaleKinematics() },
    onBalanced: (() -> Unit)? = null,
) {
  val contentHeight = size * (MARK_HEIGHT / MARK_WIDTH)

  var settleStartTime by remember { mutableStateOf<Float?>(null) }
  var hasNotifiedBalanced by remember { mutableStateOf(false) }

  val internalElapsed by
      produceState(initialValue = 0f) {
        val startNanos = withFrameNanos { it }
        while (true) {
          withFrameNanos { frameTimeNanos ->
            value = (frameTimeNanos - startNanos) / 1_000_000_000f
          }
        }
      }

  val currentElapsed = elapsedSeconds ?: internalElapsed

  LaunchedEffect(isSyncComplete) {
    if (isSyncComplete && settleStartTime == null) {
      settleStartTime = currentElapsed
    }
  }

  val settleElapsed = settleStartTime?.let { currentElapsed - it }
  val stage =
      previewStage
          ?: kinematics.evaluate(
              elapsedSinceStart = currentElapsed,
              isSyncComplete = isSyncComplete,
              settleElapsed = settleElapsed,
          )

  LaunchedEffect(stage) {
    if (stage is BalanceScaleKinematics.Stage.Balanced && !hasNotifiedBalanced) {
      hasNotifiedBalanced = true
      onBalanced?.invoke()
    }
  }

  val standPath = remember { createStandPath() }
  val beamPath = remember { createBeamPath() }
  val panPath = remember { createPanPath() }
  val coinPath = remember { createCoinPath() }
  val btcPath = remember { createBtcPath() }

  Box(modifier = modifier.size(size, contentHeight)) {
    Canvas(modifier = Modifier.matchParentSize()) {
      val sX = this.size.width / MARK_WIDTH
      val sY = this.size.height / MARK_HEIGHT

      val angleDegrees =
          when (stage) {
            is BalanceScaleKinematics.Stage.Resting,
            is BalanceScaleKinematics.Stage.Balanced,
            is BalanceScaleKinematics.Stage.Shimmer -> 0f
            is BalanceScaleKinematics.Stage.Oscillating -> stage.angle
            is BalanceScaleKinematics.Stage.Settling -> stage.angle
          }

      val rad = angleDegrees * (PI.toFloat() / 180f)
      val leftArmDX = LEFT_PIVOT_X - FULCRUM_X
      val leftPivotNewX = FULCRUM_X + leftArmDX * cos(rad)
      val leftPivotNewY = FULCRUM_Y + leftArmDX * sin(rad)
      val panShiftX = leftPivotNewX - LEFT_PIVOT_X
      val panShiftY = leftPivotNewY - LEFT_PIVOT_Y

      val shimmerBrush =
          if (stage is BalanceScaleKinematics.Stage.Shimmer) {
            val (startNorm, endNorm) = BalanceScaleKinematics.shimmerSweepRange(stage.progress)
            val startX = MARK_MIN_X + startNorm * MARK_WIDTH
            val startY = MARK_MIN_Y + startNorm * MARK_HEIGHT
            val endX = MARK_MIN_X + endNorm * MARK_WIDTH
            val endY = MARK_MIN_Y + endNorm * MARK_HEIGHT

            Brush.linearGradient(
                colorStops =
                    arrayOf(
                        0.0f to Color.Transparent,
                        0.32f to Color.White.copy(alpha = 0.20f),
                        0.50f to Color.White.copy(alpha = 0.90f),
                        0.68f to Color.White.copy(alpha = 0.20f),
                        1.0f to Color.Transparent,
                    ),
                start = Offset(startX, startY),
                end = Offset(endX, endY),
            )
          } else null

      // 1. Draw Stand
      withTransform({
        scale(sX, sY, pivot = Offset.Zero)
        translate(-MARK_MIN_X, -MARK_MIN_Y)
      }) {
        drawPath(path = standPath, color = baseColor)
        if (shimmerBrush != null) {
          drawPath(path = standPath, brush = shimmerBrush)
        }
      }

      // 2. Draw Beam, Coin, and BTC
      withTransform({
        scale(sX, sY, pivot = Offset.Zero)
        translate(-MARK_MIN_X, -MARK_MIN_Y)
        rotate(degrees = angleDegrees, pivot = Offset(FULCRUM_X, FULCRUM_Y))
      }) {
        drawPath(path = beamPath, color = baseColor)
        drawPath(path = coinPath, color = baseColor)
        drawPath(path = btcPath, color = baseColor)

        if (shimmerBrush != null) {
          drawPath(path = beamPath, brush = shimmerBrush)
          drawPath(path = coinPath, brush = shimmerBrush)
          drawPath(path = btcPath, brush = shimmerBrush)
        }
      }

      // 3. Draw Hanging Pan
      withTransform({
        scale(sX, sY, pivot = Offset.Zero)
        translate(-MARK_MIN_X, -MARK_MIN_Y)
        translate(panShiftX, panShiftY)
      }) {
        drawPath(path = panPath, color = baseColor)
        if (shimmerBrush != null) {
          drawPath(path = panPath, brush = shimmerBrush)
        }
      }
    }
  }
}

private fun createStandPath(): Path =
    Path().apply {
      fillType = PathFillType.EvenOdd
      moveTo(519.5f, 352.5f)
      lineTo(519.2f, 334.2f)
      cubicTo(519.0f, 318.2f, 519.1f, 316.0f, 520.5f, 316.0f)
      cubicTo(521.4f, 316.0f, 523.5f, 315.3f, 525.3f, 314.4f)
      cubicTo(531.8f, 311.1f, 535.8f, 306.9f, 539.2f, 300.3f)
      cubicTo(542.3f, 294.1f, 542.5f, 293.0f, 542.5f, 283.6f)
      cubicTo(542.5f, 272.3f, 540.9f, 267.4f, 535.0f, 260.7f)
      cubicTo(529.3f, 254.2f, 523.9f, 251.7f, 514.5f, 251.2f)
      cubicTo(500.1f, 250.4f, 490.1f, 255.9f, 484.3f, 267.8f)
      cubicTo(481.8f, 272.9f, 481.5f, 274.5f, 481.5f, 284.0f)
      cubicTo(481.5f, 293.1f, 481.8f, 295.2f, 483.9f, 299.5f)
      cubicTo(487.2f, 306.1f, 493.6f, 312.3f, 499.3f, 314.5f)
      lineTo(504.0f, 316.3f)
      lineTo(504.0f, 334.6f)
      lineTo(504.0f, 353.0f)
      lineTo(504.0f, 369.0f)
      lineTo(504.0f, 565.5f)
      lineTo(504.0f, 762.0f)
      lineTo(421.6f, 762.0f)
      cubicTo(348.0f, 762.0f, 339.0f, 762.2f, 337.6f, 763.6f)
      cubicTo(335.7f, 765.5f, 335.4f, 774.0f, 337.2f, 775.8f)
      cubicTo(338.1f, 776.7f, 378.2f, 777.0f, 511.6f, 777.0f)
      cubicTo(706.3f, 777.0f, 688.0f, 777.8f, 688.0f, 769.1f)
      cubicTo(688.0f, 766.5f, 687.5f, 763.9f, 686.8f, 763.2f)
      cubicTo(685.9f, 762.3f, 665.8f, 762.0f, 602.3f, 762.0f)
      lineTo(519.0f, 762.0f)
      lineTo(519.0f, 565.5f)
      lineTo(519.0f, 369.0f)
      lineTo(519.5f, 352.5f)
      close()
    }

private fun createBeamPath(): Path =
    Path().apply {
      fillType = PathFillType.EvenOdd
      moveTo(270.0f, 353.0f)
      lineTo(630.0f, 353.0f)
      lineTo(630.0f, 369.0f)
      lineTo(270.0f, 369.0f)
      close()
    }

private fun createPanPath(): Path =
    Path().apply {
      fillType = PathFillType.EvenOdd
      moveTo(273.0f, 361.0f)
      lineTo(265.1f, 353.0f)
      lineTo(262.6f, 355.2f)
      cubicTo(261.3f, 356.5f, 244.1f, 387.9f, 224.4f, 425.0f)
      cubicTo(171.9f, 524.2f, 176.0f, 516.0f, 176.0f, 521.4f)
      cubicTo(176.0f, 533.2f, 186.3f, 551.5f, 201.0f, 565.9f)
      cubicTo(207.1f, 571.8f, 223.4f, 582.4f, 230.7f, 585.1f)
      cubicTo(247.7f, 591.3f, 247.3f, 591.2f, 267.0f, 591.1f)
      cubicTo(277.2f, 591.1f, 287.8f, 590.6f, 290.5f, 590.1f)
      cubicTo(299.7f, 588.4f, 313.3f, 582.5f, 323.0f, 576.0f)
      cubicTo(330.9f, 570.7f, 345.8f, 556.0f, 349.0f, 550.2f)
      cubicTo(350.3f, 547.9f, 351.7f, 546.0f, 352.1f, 546.0f)
      cubicTo(352.5f, 546.0f, 353.2f, 545.0f, 353.6f, 543.7f)
      cubicTo(353.9f, 542.5f, 355.5f, 538.9f, 357.1f, 535.6f)
      cubicTo(359.6f, 530.7f, 360.0f, 528.6f, 360.0f, 521.8f)
      cubicTo(360.0f, 514.2f, 358.2f, 508.0f, 356.1f, 508.0f)
      cubicTo(355.6f, 508.0f, 354.9f, 506.7f, 354.5f, 505.0f)
      cubicTo(354.1f, 503.4f, 352.8f, 500.5f, 351.5f, 498.6f)
      cubicTo(350.2f, 496.7f, 348.2f, 493.2f, 347.0f, 490.8f)
      cubicTo(345.8f, 488.4f, 343.2f, 483.3f, 341.1f, 479.5f)
      cubicTo(339.0f, 475.6f, 335.9f, 469.8f, 334.1f, 466.5f)
      cubicTo(332.4f, 463.2f, 329.7f, 458.2f, 328.1f, 455.5f)
      cubicTo(326.5f, 452.7f, 324.3f, 448.7f, 323.1f, 446.5f)
      cubicTo(321.9f, 444.3f, 319.5f, 439.8f, 317.6f, 436.5f)
      cubicTo(315.8f, 433.2f, 313.1f, 428.2f, 311.6f, 425.5f)
      cubicTo(310.2f, 422.7f, 307.5f, 417.8f, 305.6f, 414.5f)
      cubicTo(303.8f, 411.2f, 300.8f, 405.8f, 299.1f, 402.5f)
      cubicTo(297.3f, 399.2f, 294.6f, 394.2f, 293.0f, 391.5f)
      cubicTo(291.4f, 388.7f, 289.1f, 384.5f, 288.0f, 382.0f)
      cubicTo(286.8f, 379.5f, 285.3f, 376.8f, 284.6f, 376.0f)
      cubicTo(283.9f, 375.2f, 282.9f, 373.3f, 282.2f, 371.7f)
      lineTo(281.0f, 369.0f)
      lineTo(273.0f, 361.0f)
      close()
      moveTo(269.8f, 386.3f)
      cubicTo(271.1f, 388.6f, 277.8f, 400.8f, 284.7f, 413.5f)
      cubicTo(291.6f, 426.1f, 301.2f, 443.7f, 306.0f, 452.5f)
      cubicTo(331.6f, 499.2f, 339.0f, 512.9f, 339.0f, 513.4f)
      cubicTo(339.0f, 513.7f, 307.0f, 514.0f, 268.0f, 514.0f)
      lineTo(197.0f, 514.0f)
      lineTo(198.2f, 511.8f)
      cubicTo(198.9f, 510.6f, 199.8f, 509.1f, 200.2f, 508.6f)
      cubicTo(200.6f, 508.0f, 206.8f, 495.8f, 214.0f, 481.5f)
      cubicTo(221.2f, 467.3f, 227.4f, 455.3f, 227.8f, 455.0f)
      cubicTo(228.2f, 454.7f, 230.5f, 450.6f, 232.7f, 445.8f)
      cubicTo(235.0f, 441.0f, 237.4f, 436.5f, 238.0f, 435.8f)
      cubicTo(238.5f, 435.1f, 242.2f, 428.2f, 246.1f, 420.5f)
      cubicTo(250.0f, 412.8f, 253.6f, 406.3f, 254.0f, 406.0f)
      cubicTo(254.4f, 405.7f, 256.6f, 401.5f, 259.0f, 396.6f)
      cubicTo(261.3f, 391.6f, 263.9f, 387.0f, 264.6f, 386.2f)
      cubicTo(265.4f, 385.5f, 266.0f, 384.2f, 266.0f, 383.4f)
      cubicTo(266.0f, 380.9f, 267.5f, 381.9f, 269.8f, 386.3f)
      moveTo(341.0f, 529.9f)
      cubicTo(341.0f, 531.9f, 330.3f, 546.8f, 325.1f, 552.1f)
      cubicTo(318.2f, 559.0f, 306.0f, 567.1f, 298.2f, 570.0f)
      cubicTo(286.3f, 574.5f, 280.9f, 575.5f, 268.5f, 575.4f)
      cubicTo(258.3f, 575.3f, 255.2f, 574.9f, 248.0f, 572.5f)
      cubicTo(237.8f, 569.3f, 228.6f, 564.6f, 220.5f, 558.5f)
      cubicTo(211.7f, 552.0f, 210.0f, 550.4f, 206.8f, 545.4f)
      cubicTo(205.1f, 543.0f, 203.4f, 541.0f, 202.9f, 541.0f)
      cubicTo(202.4f, 541.0f, 202.0f, 540.6f, 202.0f, 540.0f)
      cubicTo(202.0f, 539.5f, 200.9f, 537.4f, 199.5f, 535.4f)
      cubicTo(198.1f, 533.4f, 197.0f, 531.1f, 197.0f, 530.4f)
      cubicTo(197.0f, 529.2f, 208.9f, 529.0f, 269.0f, 529.0f)
      cubicTo(309.3f, 529.0f, 341.0f, 529.4f, 341.0f, 529.9f)
    }

private fun createCoinPath(): Path =
    Path().apply {
      fillType = PathFillType.EvenOdd
      moveTo(630.0f, 361.0f)
      lineTo(630.4f, 377.2f)
      cubicTo(633.0f, 408.4f, 651.0f, 439.6f, 678.0f, 459.7f)
      cubicTo(684.7f, 464.7f, 698.9f, 472.2f, 705.0f, 473.9f)
      cubicTo(707.5f, 474.6f, 710.4f, 475.7f, 711.5f, 476.3f)
      cubicTo(712.6f, 476.9f, 715.3f, 477.7f, 717.5f, 478.1f)
      cubicTo(719.7f, 478.5f, 723.5f, 479.4f, 726.0f, 480.0f)
      cubicTo(731.8f, 481.5f, 758.4f, 481.7f, 767.5f, 480.3f)
      cubicTo(779.8f, 478.3f, 792.5f, 473.8f, 807.4f, 466.2f)
      cubicTo(820.7f, 459.3f, 837.4f, 443.5f, 847.2f, 428.5f)
      cubicTo(850.4f, 423.5f, 853.7f, 418.6f, 854.5f, 417.5f)
      cubicTo(855.3f, 416.5f, 856.0f, 415.1f, 856.0f, 414.5f)
      cubicTo(856.0f, 413.9f, 856.9f, 411.8f, 858.0f, 409.7f)
      cubicTo(859.1f, 407.7f, 860.0f, 405.2f, 860.0f, 404.3f)
      cubicTo(860.0f, 403.4f, 860.8f, 400.8f, 861.9f, 398.6f)
      cubicTo(862.9f, 396.3f, 864.2f, 391.6f, 864.9f, 388.0f)
      cubicTo(865.5f, 384.4f, 866.5f, 379.0f, 867.0f, 376.0f)
      cubicTo(868.3f, 368.6f, 867.4f, 345.9f, 865.5f, 338.5f)
      cubicTo(864.6f, 335.2f, 863.5f, 330.7f, 863.0f, 328.5f)
      cubicTo(861.8f, 323.1f, 855.5f, 308.2f, 853.3f, 305.6f)
      cubicTo(852.3f, 304.4f, 850.6f, 301.8f, 849.6f, 299.8f)
      cubicTo(848.6f, 297.8f, 847.0f, 295.3f, 846.1f, 294.3f)
      cubicTo(845.2f, 293.3f, 843.2f, 290.6f, 841.7f, 288.4f)
      cubicTo(838.8f, 284.1f, 823.6f, 269.0f, 822.2f, 269.0f)
      cubicTo(821.7f, 269.0f, 819.9f, 267.7f, 818.0f, 266.0f)
      cubicTo(814.3f, 262.7f, 800.6f, 255.4f, 792.0f, 252.1f)
      cubicTo(783.1f, 248.7f, 772.2f, 246.2f, 761.1f, 245.1f)
      cubicTo(748.4f, 243.8f, 742.1f, 243.8f, 730.5f, 245.5f)
      cubicTo(713.6f, 247.8f, 700.6f, 252.4f, 686.7f, 260.7f)
      cubicTo(656.6f, 278.8f, 635.8f, 310.3f, 630.8f, 345.3f)
      lineTo(629.7f, 353.0f)
      lineTo(630.0f, 361.0f)
      close()
      moveTo(760.0f, 261.9f)
      cubicTo(774.6f, 264.0f, 777.1f, 264.9f, 792.5f, 272.3f)
      cubicTo(804.6f, 278.2f, 809.0f, 281.2f, 819.0f, 291.0f)
      cubicTo(833.7f, 305.3f, 842.6f, 320.5f, 847.7f, 339.9f)
      cubicTo(850.8f, 351.8f, 851.2f, 370.2f, 848.6f, 381.8f)
      cubicTo(846.2f, 392.5f, 840.9f, 405.9f, 835.4f, 415.2f)
      cubicTo(829.6f, 424.8f, 813.6f, 441.6f, 804.5f, 447.5f)
      cubicTo(772.5f, 468.3f, 736.9f, 470.6f, 703.9f, 454.0f)
      cubicTo(674.7f, 439.2f, 654.4f, 412.2f, 648.6f, 380.0f)
      cubicTo(646.3f, 367.4f, 646.6f, 345.2f, 649.1f, 342.5f)
      cubicTo(649.6f, 342.0f, 650.0f, 340.3f, 650.0f, 338.8f)
      cubicTo(650.0f, 334.7f, 654.1f, 323.6f, 659.1f, 314.1f)
      cubicTo(664.2f, 304.6f, 673.7f, 292.6f, 681.8f, 285.6f)
      cubicTo(688.8f, 279.7f, 700.3f, 272.0f, 702.4f, 272.0f)
      cubicTo(703.2f, 272.0f, 704.5f, 271.4f, 705.2f, 270.7f)
      cubicTo(707.5f, 268.4f, 719.2f, 264.7f, 730.0f, 262.9f)
      cubicTo(734.1f, 262.2f, 738.2f, 261.3f, 739.0f, 260.8f)
      cubicTo(740.6f, 259.9f, 749.2f, 260.4f, 760.0f, 261.9f)
    }

private fun createBtcPath(): Path =
    Path().apply {
      fillType = PathFillType.EvenOdd
      moveTo(750.0f, 288.0f)
      cubicTo(748.3f, 289.7f, 748.0f, 291.3f, 748.0f, 298.5f)
      lineTo(748.0f, 307.0f)
      lineTo(745.0f, 307.0f)
      lineTo(742.0f, 307.0f)
      lineTo(742.0f, 298.6f)
      cubicTo(742.0f, 288.1f, 741.2f, 287.0f, 733.0f, 287.0f)
      cubicTo(724.8f, 287.0f, 724.0f, 288.1f, 724.0f, 298.6f)
      lineTo(724.0f, 307.0f)
      lineTo(713.9f, 307.0f)
      cubicTo(708.4f, 307.0f, 703.0f, 307.4f, 702.0f, 308.0f)
      cubicTo(698.4f, 310.0f, 696.8f, 318.9f, 699.6f, 322.5f)
      cubicTo(700.8f, 324.1f, 702.6f, 324.6f, 709.3f, 325.0f)
      lineTo(717.5f, 325.5f)
      lineTo(717.8f, 364.2f)
      lineTo(718.0f, 403.0f)
      lineTo(710.6f, 403.0f)
      cubicTo(701.2f, 403.0f, 700.0f, 404.0f, 700.0f, 412.0f)
      cubicTo(700.0f, 420.4f, 700.8f, 421.0f, 713.6f, 421.0f)
      lineTo(724.0f, 421.0f)
      lineTo(724.0f, 429.9f)
      cubicTo(724.0f, 440.8f, 724.8f, 442.0f, 732.3f, 442.0f)
      cubicTo(735.2f, 442.0f, 738.0f, 441.6f, 738.6f, 441.2f)
      cubicTo(741.0f, 439.7f, 742.0f, 435.8f, 742.0f, 428.6f)
      lineTo(742.0f, 421.0f)
      lineTo(745.0f, 421.0f)
      lineTo(748.0f, 421.0f)
      lineTo(748.0f, 429.4f)
      cubicTo(748.0f, 437.0f, 748.2f, 438.0f, 750.2f, 439.4f)
      cubicTo(751.6f, 440.4f, 754.6f, 441.0f, 757.7f, 441.0f)
      cubicTo(765.6f, 441.0f, 767.0f, 439.1f, 767.0f, 429.1f)
      lineTo(767.0f, 421.1f)
      lineTo(772.8f, 420.0f)
      cubicTo(780.9f, 418.4f, 783.8f, 417.3f, 789.5f, 413.9f)
      cubicTo(794.6f, 410.8f, 797.7f, 406.1f, 799.5f, 398.7f)
      cubicTo(799.9f, 397.2f, 800.6f, 395.7f, 801.1f, 395.4f)
      cubicTo(802.3f, 394.7f, 802.2f, 380.3f, 801.0f, 379.5f)
      cubicTo(800.5f, 379.2f, 800.0f, 377.8f, 800.0f, 376.4f)
      cubicTo(800.0f, 372.9f, 797.5f, 368.4f, 792.4f, 363.0f)
      cubicTo(789.4f, 359.8f, 788.2f, 357.7f, 788.6f, 356.6f)
      cubicTo(788.9f, 355.7f, 789.6f, 355.0f, 790.1f, 355.0f)
      cubicTo(793.1f, 355.0f, 794.9f, 331.5f, 792.4f, 325.4f)
      cubicTo(790.9f, 321.9f, 788.3f, 318.5f, 784.0f, 314.3f)
      cubicTo(781.4f, 311.8f, 772.3f, 308.0f, 769.0f, 308.0f)
      cubicTo(767.2f, 308.0f, 767.0f, 307.3f, 767.0f, 299.2f)
      cubicTo(767.0f, 294.4f, 766.6f, 290.0f, 766.2f, 289.4f)
      cubicTo(763.7f, 285.6f, 753.3f, 284.7f, 750.0f, 288.0f)
      moveTo(766.6f, 328.0f)
      cubicTo(769.5f, 329.4f, 771.4f, 331.4f, 773.0f, 334.4f)
      cubicTo(775.6f, 339.2f, 775.3f, 342.1f, 771.9f, 346.4f)
      cubicTo(768.0f, 351.4f, 764.8f, 352.3f, 749.5f, 352.4f)
      lineTo(735.5f, 352.5f)
      lineTo(735.2f, 339.2f)
      lineTo(734.9f, 326.0f)
      lineTo(748.7f, 326.0f)
      cubicTo(760.4f, 326.0f, 763.0f, 326.3f, 766.6f, 328.0f)
      moveTo(776.5f, 376.0f)
      cubicTo(783.6f, 382.8f, 784.6f, 388.5f, 780.0f, 395.0f)
      cubicTo(775.6f, 401.2f, 771.4f, 402.3f, 752.3f, 402.8f)
      lineTo(735.0f, 403.2f)
      lineTo(735.0f, 387.5f)
      lineTo(735.0f, 371.8f)
      lineTo(753.9f, 372.2f)
      lineTo(772.9f, 372.5f)
      lineTo(776.5f, 376.0f)
    }
