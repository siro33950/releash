import Bonsplit
import SwiftUI

struct PaneOpenMenu: View {
  @Bindable var model: AppModel
  let pane: PaneID
  var body: some View {
    Menu {
      Button("Terminal", systemImage: "terminal") { model.panes.open(.terminal, in: pane) }
    } label: {
      Image(systemName: "plus")
    }
    .menuStyle(.button)
    .buttonStyle(.accessoryBar)
    .disabled(!model.layoutLoaded)
    .menuIndicator(.hidden)
    .frame(width: UIStyle.iconButtonWidth, height: UIStyle.iconButtonHeight)
    .fixedSize()
    .padding(.horizontal, UIStyle.spacing)
    .help("開く")
    .accessibilityLabel("開く")
  }
}
