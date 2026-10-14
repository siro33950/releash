import Bonsplit
import SwiftUI

struct PaneOpenMenu: View {
  @Bindable var model: AppModel
  let pane: PaneID
  var body: some View {
    IconMenu(title: "開く", systemImage: "plus") {
      Button("Terminal", systemImage: "terminal") { model.panes.open(.terminal, in: pane) }
    }
    .disabled(!model.layoutLoaded)
    .padding(.horizontal, UIStyle.spacing)
  }
}
