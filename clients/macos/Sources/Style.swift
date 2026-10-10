import SwiftUI

enum UIStyle {
  static let terminalRows: UInt32 = 24
  static let terminalColumns: UInt32 = 80
  static let spacing: CGFloat = 8
  static let padding: CGFloat = 12
  static let sidebarWidth: CGFloat = 280
  static let sidebarMaximumWidth: CGFloat = 400
  static let selectionListHeight: CGFloat = 160
  static let tabBarHeight: CGFloat = 32
  static let tabCloseSize: CGFloat = 14
  static let tabDragThreshold: CGFloat = 4
  static let tabSpacing: CGFloat = 4
  static let tabPadding: CGFloat = 4
  static let tabLeadingSlot: CGFloat = 12
  static let tabFontSize: CGFloat = 13
  static let tabBorder: CGFloat = 1
  static let tabInsertionBorder: CGFloat = 2
  static let tabControlWidth: CGFloat = 28
  static let switcherWidth: CGFloat = 300
  static let dropOpacity: Double = 0.18
  static let splitDropFraction: CGFloat = 0.2
  static let footerHeight: CGFloat = 24
  static let statusSize: CGFloat = 8
  static let minimumPaneSize: CGFloat = 240
  static let attention = Color.yellow
  static let active = Color.blue
  static let idle = Color.green
  static let addition = Color.green
  static let deletion = Color.red
  static let cardShape = RoundedRectangle(cornerRadius: 8)

  static func statusColor(_ status: String) -> Color {
    switch status {
    case "attention": attention
    case "active": active
    default: idle
    }
  }
}
