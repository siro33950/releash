import SwiftUI

enum UIStyle {
  static let terminalRows: UInt32 = 24
  static let terminalColumns: UInt32 = 80
  static let spacing: CGFloat = 8
  static let padding: CGFloat = 12
  static let rowSpacing: CGFloat = 4
  static let statusSpacing: CGFloat = 3
  static let cardIconWidth: CGFloat = 14
  static let sidebarWidth: CGFloat = 280
  static let rightPanelMinimumWidth: CGFloat = 200
  static let dividerWidth: CGFloat = 1
  static let dividerCursorWidth: CGFloat = 8
  static let sidebarMaximumWidth: CGFloat = 400
  static let createDialogWidth: CGFloat = 560
  static let createDialogHeight: CGFloat = 560
  static let selectionListHeight: CGFloat = 160
  static let windowWidth: CGFloat = 1200
  static let windowHeight: CGFloat = 900
  static let windowMinimumWidth: CGFloat = 720
  static let windowMinimumHeight: CGFloat = 480
  static let windowPadding: CGFloat = 8
  static let contentCornerRadius: CGFloat = 12
  static let windowCornerRadius: CGFloat = 20
  static let windowControlsWidth: CGFloat = 68
  static let windowControlSpacing: CGFloat = 22
  static let tabBarHeight: CGFloat = 33
  static let iconButtonWidth: CGFloat = 28
  static let iconButtonHeight: CGFloat = 24
  static let collapsedPanelWidth: CGFloat = 36
  static let footerHeight: CGFloat = 24
  static let statusSize: CGFloat = 8
  static let tabSelection = Color(nsColor: .secondaryLabelColor)
  static let sidebarSelection = Color(nsColor: .selectedContentBackgroundColor)
  static let sidebarSelectionText = Color(nsColor: .alternateSelectedControlTextColor)
  static let iconButtonCornerRadius: CGFloat = 5
  static let attention = Color.yellow
  static let active = Color.blue
  static let idle = Color.green
  static let addition = Color.green
  static let deletion = Color.red

  static func statusColor(_ status: String) -> Color {
    switch status {
    case "attention": attention
    case "active": active
    default: idle
    }
  }
}

/// アイコンだけのトグル。off は accessoryBar のボタンと同じ見た目で、on は worktree の選択と同じ色にする
struct IconToggleStyle: ToggleStyle {
  func makeBody(configuration: Configuration) -> some View {
    Button { configuration.isOn.toggle() } label: {
      configuration.label.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    .buttonStyle(.accessoryBar)
    .background(
      configuration.isOn ? UIStyle.sidebarSelection : .clear,
      in: RoundedRectangle(cornerRadius: UIStyle.iconButtonCornerRadius))
    .foregroundStyle(configuration.isOn ? UIStyle.sidebarSelectionText : .primary)
  }
}
