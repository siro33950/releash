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
struct WorkflowLaunchFields: View {
  let workflows: [Releash_Client_V1_WorkflowSummaryDto]
  @Binding var workflow: String
  @Binding var requestText: String
  var body: some View {
    Picker("Workflow", selection: $workflow) {
      Text("選択してください").tag("")
      ForEach(workflows, id: \.name) { Text($0.name).tag($0.name) }
    }
    TextField("依頼文", text: $requestText, axis: .vertical)
  }
}
