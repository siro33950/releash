import SwiftUI

struct TerminalPane: View {
  @Bindable var session: TerminalSession
  let branch: String
  let focused: Bool

  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Text(session.cwd ?? session.path).lineLimit(1).truncationMode(.middle)
        Spacer()
        Label(branch, systemImage: "arrow.triangle.branch")
        Button("Clear") { session.clear() }.buttonStyle(.borderless)
      }
      .font(.caption)
      .padding(UIStyle.spacing)
      if let failure = session.failure {
        Text(failure).foregroundStyle(.red).textSelection(.enabled)
      }
      TerminalDisplay(session: session, focused: focused)
    }
  }
}

struct TerminalDisplay: NSViewRepresentable {
  let session: TerminalSession
  let focused: Bool
  func makeNSView(context: Context) -> TerminalScreen {
    session.screen.setFocused(focused)
    return session.screen
  }
  func updateNSView(_ view: TerminalScreen, context: Context) { view.setFocused(focused) }
}
