import SwiftUI

/// Step 2: Amount entry, fiat/sat conversions, LNURL constraints, and optional payee comment.
struct SendAmountStepView: View {
    @Bindable var model: SendFlowModel
    @Environment(AppState.self) private var appState
    @FocusState private var isAmountFocused: Bool

    var body: some View {
        ScrollView(.vertical, showsIndicators: false) {
            VStack(spacing: 16) {
                if let payeeInfo = model.lnurlParams?.plainTextDescription {
                    payeeMetadataCard(description: payeeInfo)
                }
                heroAmountCard
                availableBalanceCard
                presetPercentages
                if let params = model.lnurlParams, let maxComment = params.commentAllowed, maxComment > 0 {
                    commentCard(maxCharacters: maxComment)
                }
                if let error = model.errorMessage {
                    SendErrorBanner(message: error)
                }
                Spacer(minLength: 24)
                continueButton
            }
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 16)
            .padding(.top, 16)
        }
        .scrollDismissesKeyboard(.immediately)
        .scrollBounceBehavior(.basedOnSize)
        .contentShape(Rectangle())
        .onTapGesture {
            isAmountFocused = false
            UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        }
        .onAppear {
            isAmountFocused = true
            model.refreshMaxAmountText(appState: appState)
        }
        .onChange(of: isAmountFocused) { _, isFocused in
            if !isFocused { model.normalizeAmountInput() }
        }
        .task {
            guard case .onchain = model.destination, model.feeRateSatVb == nil else { return }
            await model.applyRecommendedFees(appState.feeRateService.recommendedFees(), appState: appState)
        }
    }

    private var availableBalanceCard: some View {
        let available = model.availableSpendableSats(appState: appState)
        let sats = model.sendAmountSats(appState: appState)
        // Fee-aware decision comes from the model; `sats > available` only picks the wording.
        let isInsufficient = sats > 0 && model.isInsufficientBalance(appState: appState)
        let insufficientLabel = sats > available
            ? String(localized: .errorInsufficientFunds)
            : String(localized: .errorInsufficientFundsInclFee)
        let usd = (Double(available) / Double(Constants.satsInBTC)) * appState.accountingBTCPrice

        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(String(localized: .headerAvailableBalance))
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                if isInsufficient {
                    Text(insufficientLabel)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.red)
                }
            }

            HStack {
                Text(verbatim: "\(available.btcSpacedFormatted) BTC")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(isInsufficient ? Color.red : Color.primary)
                Spacer()
                if appState.accountingBTCPrice > 0 {
                    Text(verbatim: "≈ \(usd.usdFormatted)")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .sendCard()
    }

    private var heroAmountCard: some View {
        VStack(spacing: 12) {
            HStack {
                unitMenuButton
                Spacer()
            }

            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if model.amountUnit == .usd {
                    Text(model.amountUnit.symbolOrSuffix)
                        .font(.system(size: 34, weight: .bold, design: .rounded))
                        .foregroundStyle(.secondary)
                }

                ZStack {
                    Text(model.amountInputText.isEmpty ? model.amountUnit.placeholder : model.amountInputText)
                        .font(.system(size: 38, weight: .bold, design: .rounded).monospacedDigit())
                        .foregroundStyle(
                            isAmountFocused
                                ? Color.clear
                                : (model.amountInputText.isEmpty ? Color.secondary.opacity(0.4) : Color.primary)
                        )
                        .contentTransition(.numericText())
                        .padding(.horizontal, 4)
                        .frame(minWidth: 44)

                    TextField(
                        model.amountUnit.placeholder,
                        text: $model.amountInputText
                    )
                    .font(.system(size: 38, weight: .bold, design: .rounded).monospacedDigit())
                    .keyboardType(model.amountUnit == .sats ? .numberPad : .decimalPad)
                    .multilineTextAlignment(.center)
                    .lineLimit(1)
                    .focused($isAmountFocused)
                    .foregroundStyle(isAmountFocused ? Color.primary : Color.clear)
                }
                .fixedSize(horizontal: true, vertical: false)

                if model.amountUnit != .usd {
                    Text(model.amountUnit.symbolOrSuffix)
                        .font(.system(size: 20, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .center)
            .contentShape(Rectangle())
            .onTapGesture { isAmountFocused = true }
            .onChange(of: model.amountInputText) { _, new in
                let sanitized = InputSanitizer.decimal(new, maxDecimals: model.amountUnit.maxDecimals)
                if sanitized.count > 16 {
                    model.amountInputText = String(sanitized.prefix(16))
                } else if sanitized != new {
                    model.amountInputText = sanitized
                }
                if isAmountFocused { model.amountEdited(appState: appState) }
            }

            let sats = model.computeEffectiveSats(btcPrice: appState.accountingBTCPrice)
            secondaryConversionButton(sats: sats)
                .opacity(sats > 0 ? 1.0 : 0.45)
                .allowsHitTesting(sats > 0)

            if let params = model.lnurlParams, params.hasCustomSendBounds {
                Text(model.amountUnit.allowedRangeText(params: params, btcPrice: appState.accountingBTCPrice))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .sendCard(padding: 18)
    }

    private var unitMenuButton: some View {
        Menu {
            ForEach(SendAmountUnit.allCases) { unit in
                Button {
                    UISelectionFeedbackGenerator().selectionChanged()
                    model.switchUnit(to: unit, btcPrice: appState.accountingBTCPrice)
                } label: {
                    Text(unit.menuTitle)
                }
            }
        } label: {
            HStack(spacing: 4) {
                Text(model.amountUnit.title).font(.subheadline.weight(.semibold))
                Image(systemName: "chevron.up.chevron.down").font(.caption2.weight(.bold))
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Color(uiColor: .tertiarySystemFill), in: Capsule())
            .foregroundStyle(.primary)
        }
    }

    private func secondaryConversionButton(sats: UInt64) -> some View {
        Button {
            UISelectionFeedbackGenerator().selectionChanged()
            let nextUnit: SendAmountUnit = model.amountUnit == .usd ? .sats : .usd
            model.switchUnit(to: nextUnit, btcPrice: appState.accountingBTCPrice)
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "arrow.up.arrow.down").font(.caption2.weight(.semibold))
                Text(model.amountUnit.secondaryConversionText(sats: sats, btcPrice: appState.accountingBTCPrice))
                    .font(.subheadline.weight(.medium))
            }
            .foregroundStyle(.secondary)
            .contentTransition(.numericText())
        }
        .buttonStyle(.plain)
    }

    private func payeeMetadataCard(description: String) -> some View {
        HStack(spacing: 12) {
            if let avatar = model.payeeAvatar {
                Image(decorative: avatar, scale: 1)
                    .resizable()
                    .scaledToFill()
                    .frame(width: 36, height: 36)
                    .clipShape(Circle())
            } else {
                Image(systemName: "person.crop.circle.fill")
                    .font(.title2)
                    .foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(String(localized: .headerPayee))
                    .font(.caption).foregroundStyle(.secondary)
                Text(description).font(.subheadline.weight(.medium)).lineLimit(2)
            }
            Spacer()
        }
        .sendCard()
    }

    private var presetPercentages: some View {
        let available = model.availableSpendableSats(appState: appState)
        return HStack(spacing: 12) {
            ForEach([25, 50, 100], id: \.self) { pct in
                Button {
                    isAmountFocused = false
                    UISelectionFeedbackGenerator().selectionChanged()
                    withAnimation(.snappy(duration: 0.32, extraBounce: 0.04)) {
                        model.applyPercentage(
                            pct,
                            totalBalanceSats: available,
                            btcPrice: appState.accountingBTCPrice,
                            appState: appState
                        )
                    }
                } label: {
                    if pct == 100 {
                        HStack(spacing: 6) {
                            LemniscateBloomIcon(isActive: model.isMaxAmount, size: 16, tint: Color.blue)
                            Text(String(localized: .buttonMax))
                                .font(.subheadline.weight(.medium))
                        }
                        .frame(maxWidth: .infinity)
                    } else {
                        Text(verbatim: "\(pct)%")
                            .font(.subheadline.weight(.medium))
                            .frame(maxWidth: .infinity)
                    }
                }
                .buttonStyle(.bordered)
                .disabled(available == 0)
            }
        }
    }

    private func commentCard(maxCharacters: Int) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(String(localized: .headerComment))
                    .font(.caption.weight(.medium)).foregroundStyle(.secondary)
                Spacer()
                Text(verbatim: "\(model.lnurlComment.count)/\(maxCharacters)").font(.caption2)
                    .foregroundStyle(.secondary)
            }
            TextField(
                String(localized: .placeholderOptionalComment),
                text: $model.lnurlComment
            )
            .font(.subheadline)
            .onChange(of: model.lnurlComment) { _, new in
                if new.count > maxCharacters { model.lnurlComment = String(new.prefix(maxCharacters)) }
            }
        }
        .sendCard()
    }

    private var continueButton: some View {
        let sats = model.sendAmountSats(appState: appState)
        let isBlocked = sats == 0 || model.isInsufficientBalance(appState: appState)

        return Button {
            isAmountFocused = false
            UIApplication.shared.sendAction(
                #selector(UIResponder.resignFirstResponder),
                to: nil,
                from: nil,
                for: nil
            )
            model.proceedFromAmount(appState: appState)
        } label: {
            Text(String(localized: .buttonContinue))
                .font(.headline).frame(maxWidth: .infinity)
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .tint(Color.blue)
        .disabled(isBlocked)
        .padding(.bottom, 16)
    }
}
