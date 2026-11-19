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
  static let settingsWidth: CGFloat = 1024
  static let settingsHeight: CGFloat = 800
  static let settingsSidebarWidth: CGFloat = 200
  static let worktreeDialogWidth = settingsWidth
  static let worktreeDialogHeight = settingsHeight
  static let historyWidth = settingsWidth
  static let historyHeight = settingsHeight
  static let createDialogWidth: CGFloat = 560
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
  static let snackbarMaximumWidth: CGFloat = 480
  static let snackbarShadowRadius: CGFloat = 4
  static let snackbarLimit = 3
  static let nodeNameMinimumWidth: CGFloat = 140
  static let nodeNameWidth: CGFloat = 240
  static let nodeMetricWidth: CGFloat = 52
  static let nodeModelMinimumWidth: CGFloat = 72
  static let nodeModelWidth: CGFloat = 112
  static let nodeTimelineMinimumWidth: CGFloat = 80
  static let nodeTimelineWidth: CGFloat = 200
  static let nodeUpperMinimumHeight: CGFloat = 160
  static let nodeDetailMinimumHeight: CGFloat = 150
  static let nodeDetailHeightRatio: CGFloat = 1.0 / 3
  static let nodeEventsWidthRatio: CGFloat = 1.0 / 3
  static let artifactFieldMinimumWidth: CGFloat = 150
  static let statusSize: CGFloat = 8
  static let tabSelection = Color(nsColor: .secondaryLabelColor)
  static let sidebarSelection = Color(nsColor: .selectedContentBackgroundColor)
  static let sidebarSelectionText = Color(nsColor: .alternateSelectedControlTextColor)
  static let iconButtonCornerRadius: CGFloat = 5
  static let iconButtonHover = Color(nsColor: .quaternarySystemFill)
  static let cardCornerRadius: CGFloat = 6
  static let badgeCornerRadius: CGFloat = 3
  static let avatarSize: CGFloat = 20
  static let commentInputLines = 6
  static let diffLineNumberWidth: CGFloat = 32
  static let diffSignWidth: CGFloat = 16
  static let diffLineSpacing: CGFloat = 2
  static let diffTargetMarkerWidth: CGFloat = 3
  static let diffTintOpacity = 0.15
  static let diffAddition = Color.green
  static let diffDeletion = Color.red
  static let attention = Color.yellow
  static let active = Color.blue
  static let idle = Color.green
  static let failure = Color.red

  static func prAppearance(_ value: String) -> (icon: String, color: Color) {
    switch value {
    case "open": ("git-pull-request", .green)
    case "draft": ("git-pull-request-draft", .gray)
    case "merged": ("git-merge", .purple)
    default: ("git-pull-request-closed", .red)
    }
  }

  static func checkAppearance(_ bucket: Releash_Client_V1_PullRequestCheckBucket) -> (
    icon: String, color: Color
  ) {
    switch bucket {
    case .fail: ("circle-alert", .red)
    case .cancel: ("triangle-alert", .orange)
    default: ("refresh-cw", .yellow)
    }
  }

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
  let image: String
  @ViewBuilder let content: () -> Content
  var body: some View {
    Menu {
      content().labelStyle(.titleAndIcon)
    } label: {
      CardIcon(image: Image(image))
    }
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

struct CardIcon: View {
  let image: Image

  var body: some View {
    image.resizable().renderingMode(.template).scaledToFit()
      .frame(width: UIStyle.cardIconWidth, height: UIStyle.cardIconWidth)
  }
}

struct CardLabel: View {
  let title: String
  let image: String

  var body: some View {
    Label {
      Text(title)
    } icon: {
      CardIcon(image: Image(image))
    }
  }
}

extension UIStyle {
  enum NodeKind {
    case session, command, sequence, fanout

    var icon: String {
      switch self {
      case .session: "bot"
      case .command: "terminal"
      case .sequence: "waypoints"
      case .fanout: "git-fork"
      }
    }
    var title: String {
      switch self {
      case .session: "Session"
      case .command: "Command"
      case .sequence: "Sequence"
      case .fanout: "Fanout"
      }
    }
    /// Command は model を持たないので、Model の欄には実行する場所を出す
    var modelLabel: String? { self == .command ? "shell" : nil }
  }
}
