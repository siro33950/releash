import Foundation
import SwiftUI
import UniformTypeIdentifiers

/// Custom UTTypes for tab drag and drop
extension UTType {
    static var tabItem: UTType {
        UTType(exportedAs: "com.splittabbar.tabitem")
    }

    static var tabTransfer: UTType {
        UTType(exportedAs: "com.splittabbar.tabtransfer")
    }
}

/// Represents a single tab in a pane's tab bar (internal representation)
struct TabItem: Identifiable, Hashable, Codable {
    let id: UUID
    var title: String
    var icon: String?
    var iconTint: TabIconTint?
    var iconColor: Color? { iconTint.map { Color(red: $0.red, green: $0.green, blue: $0.blue, opacity: $0.opacity) } }
    var isDirty: Bool

    init(
        id: UUID = UUID(),
        title: String,
        icon: String? = "doc.text",
        isDirty: Bool = false,
        iconColor: Color? = nil
    ) {
        self.id = id
        self.title = title
        self.icon = icon
        self.isDirty = isDirty
        self.iconTint = iconColor.map(TabIconTint.init)
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(id)
    }

    static func == (lhs: TabItem, rhs: TabItem) -> Bool {
        lhs.id == rhs.id
    }
}

// MARK: - Transferable for Drag & Drop

extension TabItem: Transferable {
    static var transferRepresentation: some TransferRepresentation {
        CodableRepresentation(contentType: .tabItem)
    }
}

/// Transfer data that includes source pane information for cross-pane moves
struct TabTransferData: Codable, Transferable {
    let tab: TabItem
    let sourcePaneId: UUID

    static var transferRepresentation: some TransferRepresentation {
        CodableRepresentation(contentType: .tabTransfer)
    }
}

struct TabIconTint: Codable, Hashable {
    var red: Double
    var green: Double
    var blue: Double
    var opacity: Double

    init(_ color: Color) {
        let value = NSColor(color).usingColorSpace(.sRGB)!
        red = value.redComponent
        green = value.greenComponent
        blue = value.blueComponent
        opacity = value.alphaComponent
    }
}
