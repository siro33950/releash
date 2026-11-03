import SwiftUI

struct TerminalPane: View {
  @Bindable var session: TerminalSession
  let focused: Bool

  var body: some View {
    VStack(spacing: 0) {
      if let failure = session.failure {
        Text(failure).foregroundStyle(.red).textSelection(.enabled)
      }
      if let failure = session.dropFailure {
        HStack {
          Text(failure).foregroundStyle(.red).textSelection(.enabled)
          Button("閉じる") { session.dropFailure = nil }
        }
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
