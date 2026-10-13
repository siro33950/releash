import SwiftUI

struct FooterView: View {
  @Bindable var model: AppModel
  var body: some View {
    TimelineView(.periodic(from: .now, by: 1)) { context in
      HStack(spacing: UIStyle.spacing) {
        ForEach(["claude", "codex"], id: \.self) { provider in
          let usage = model.providerUsage.first { $0.provider == provider }
          let warnings = model.hookHealth.warnings.items.filter {
            $0.provider.value == (provider == "claude" ? .claude : .codex)
          }
          if usage != nil || !warnings.isEmpty {
            ProviderFooterView(provider: provider, usage: usage, warnings: warnings, now: context.date)
          }
        }
        if !model.hookHealth.readErrors.isEmpty || model.hookSubscriptionFailure != nil {
          Image(systemName: "exclamationmark.triangle.fill")
            .foregroundStyle(UIStyle.attention)
            .help(([model.hookSubscriptionFailure].compactMap { $0 } + model.hookHealth.readErrors).joined(separator: "\n"))
            .accessibilityLabel("hookの状態を確認できません")
        }
        Spacer(minLength: 0)
        if let info = model.daemonInfo {
          Menu {
            if model.releaseCompatibility == "server_older" {
              Text("サーバが古いため、再起動で更新できます")
            } else if model.releaseCompatibility == "client_older" {
              Text("クライアントが古いです")
            }
            Button("サーバを停止") { Task { await model.stopDaemon() } }
            Button("サーバを再起動") { Task { await model.restartDaemon() } }
          } label: {
            HStack(spacing: UIStyle.statusSpacing) {
              Image(systemName: model.releaseCompatibility == "compatible" ? "server.rack" : "exclamationmark.triangle")
              Text("v\(info.release) · \(String(describing: info.servingStatus)) · \(uptime(info, now: context.date))")
              Text(model.releaseCompatibility ?? "")
            }
          }
          .menuStyle(.borderlessButton)
          .fixedSize()
          .disabled(model.serverOperation)
          .accessibilityLabel("v\(info.release) · \(model.releaseCompatibility ?? "") · \(String(describing: info.servingStatus)) · \(uptime(info, now: context.date))")
          .accessibilityIdentifier("footer-server")
          if model.releaseCompatibility == "server_older" {
            Text("再起動で更新できます").foregroundStyle(UIStyle.attention)
          } else if model.releaseCompatibility == "client_older" {
            Text("クライアントが古いです").foregroundStyle(UIStyle.attention)
          }
        }
      }
      .font(.caption)
      .padding(.horizontal, UIStyle.padding)
      .frame(height: UIStyle.footerHeight)
      .background(Color(nsColor: .controlBackgroundColor))
    }
  }

  private func uptime(_ info: Releash_Client_V1_ServerInfo, now: Date) -> String {
    let duration = max(0, Int(now.timeIntervalSince1970) - Int(info.processStartedAt))
    return "\(duration / 3600)h \((duration % 3600) / 60)m"
  }
}

private struct ProviderFooterView: View {
  let provider: String
  let usage: Releash_Client_V1_ProviderUsageEntry?
  let warnings: [Releash_Client_V1_ProviderHookHealthWarningResponse]
  let now: Date
  @State private var expanded = false
  var body: some View {
    Button { expanded.toggle() } label: {
      HStack(spacing: UIStyle.statusSpacing) {
        Image(systemName: provider == "claude" ? "sparkles" : "terminal")
          .frame(width: UIStyle.cardIconWidth, height: UIStyle.cardIconWidth)
        Text(provider.capitalized)
        if let usage {
          ForEach(Array(usage.windows.enumerated()), id: \.offset) { _, window in
            ProgressView(value: window.usedPercent, total: 100).frame(width: 28)
            Text("\(window.usedPercent, specifier: "%.0f")% \(windowLabel(window.windowMinutes))")
          }
        }
        if !warnings.isEmpty || usage?.windows.contains(where: { $0.warning }) == true {
          Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(UIStyle.attention)
        }
      }
    }
    .buttonStyle(.plain)
    .accessibilityElement(children: .ignore)
    .accessibilityAddTraits(.isButton)
    .accessibilityAction { expanded.toggle() }
    .accessibilityLabel(providerLabel)
    .accessibilityIdentifier("footer-provider:\(provider)")
    .popover(isPresented: $expanded) {
      VStack(alignment: .leading, spacing: UIStyle.spacing) {
        Text(provider.capitalized).font(.headline)
        if let usage {
          ForEach(Array(usage.windows.enumerated()), id: \.offset) { _, window in
            Text("\(windowLabel(window.windowMinutes)): resetまで\(reset(window.resetsAt))")
          }
          if usage.hasPlan { Text(usage.plan) }
        }
        ForEach(Array(warnings.enumerated()), id: \.offset) { _, warning in
          Text(warning.reason)
        }
      }.padding(UIStyle.padding)
    }
  }

  private var providerLabel: String {
    let summary = usage?.windows.map {
      String(format: "%.0f%% %@", $0.usedPercent, windowLabel($0.windowMinutes))
    }.joined(separator: " · ") ?? ""
    let usageWarning = usage?.windows.contains(where: { $0.warning }) == true ? " 使用率の警告" : ""
    let hookWarning = warnings.isEmpty ? "" : " hook警告"
    return "\(provider) \(summary)\(usageWarning)\(hookWarning)"
  }

  private func windowLabel(_ minutes: UInt64) -> String {
    if minutes == 10080 { return "wk" }
    if minutes % 60 == 0 { return "\(minutes / 60)h" }
    return "\(minutes)m"
  }

  private func reset(_ epoch: UInt64) -> String {
    let seconds = max(0, Int(epoch) - Int(now.timeIntervalSince1970))
    return "\(seconds / 3600)時間\((seconds % 3600) / 60)分"
  }
}
