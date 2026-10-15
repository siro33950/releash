import SwiftUI

enum UIStyle {
  static let terminalRows: UInt32 = 24
  static let terminalColumns: UInt32 = 80
  static let terminalFontName = "Sarasa-Term-J-Regular"
  static let spacing: CGFloat = 8
  static let padding: CGFloat = 12
  static let rowSpacing: CGFloat = 4
  static let statusSpacing: CGFloat = 3
  static let cardIconWidth: CGFloat = 14
  static let sidebarWidth: CGFloat = 280
  static let centerMinimumWidth = (windowMinimumWidth - sidebarWidth) / 2
  static let rightPanelMinimumWidth = centerMinimumWidth
  static let centerInitialWidth = (windowWidth - windowPadding * 2 - sidebarWidth) / 2
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
  static let iconButtonHover = Color(nsColor: .quaternarySystemFill)
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

extension View {
  /// アイコンだけのボタン・トグル・メニューの大きさを揃える
  func iconButtonFrame() -> some View {
    frame(width: UIStyle.iconButtonWidth, height: UIStyle.iconButtonHeight).fixedSize()
  }
}

/// アイコンだけのボタンとメニュー。マウスを乗せたときと押したときに、枠いっぱいに背景を描く
struct IconButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    HoverLabel(configuration: configuration)
  }
  private struct HoverLabel: View {
    let configuration: Configuration
    @Environment(\.isEnabled) private var enabled
    @State private var hovering = false
    var body: some View {
      configuration.label.frame(maxWidth: .infinity, maxHeight: .infinity).contentShape(Rectangle())
        .background(
          enabled && (hovering || configuration.isPressed) ? UIStyle.iconButtonHover : .clear,
          in: RoundedRectangle(cornerRadius: UIStyle.iconButtonCornerRadius)
        )
        .opacity(enabled ? 1 : 0.4)
        .onHover { hovering = $0 }
    }
  }
}

/// アイコンだけのメニュー
struct IconMenu<Content: View>: View {
  let title: String
  let systemImage: String
  @ViewBuilder let content: () -> Content
  var body: some View {
    Menu(content: content) { Image(systemName: systemImage) }
      .menuStyle(.button).buttonStyle(IconButtonStyle()).menuIndicator(.hidden)
      .iconButtonFrame()
      .help(title).accessibilityLabel(title)
  }
}

/// アイコンだけのトグル。off は IconButtonStyle と同じ見た目で、on は worktree の選択と同じ色にする
struct IconToggleStyle: ToggleStyle {
  func makeBody(configuration: Configuration) -> some View {
    Button {
      configuration.isOn.toggle()
    } label: {
      configuration.label
    }
    .buttonStyle(IconButtonStyle())
    .background(
      configuration.isOn ? UIStyle.sidebarSelection : .clear,
      in: RoundedRectangle(cornerRadius: UIStyle.iconButtonCornerRadius)
    )
    .foregroundStyle(configuration.isOn ? UIStyle.sidebarSelectionText : .primary)
  }
}
