import SwiftUI

extension Releash_Client_V1_AgentSessionProviderDto {
  var displayName: String {
    switch value {
    case .claude: "Claude"
    case .codex: "Codex"
    default: "Unknown"
    }
  }
  var wireName: String {
    switch value {
    case .claude: "claude"
    case .codex: "codex"
    default: ""
    }
  }
}
