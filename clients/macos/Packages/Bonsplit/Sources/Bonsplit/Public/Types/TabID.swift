import Foundation

/// Opaque identifier for tabs
public struct TabID: Hashable, Codable, Sendable {
    public let id: UUID

    public init() {
        self.id = UUID()
    }

    public init(id: UUID) {
        self.id = id
    }
}
