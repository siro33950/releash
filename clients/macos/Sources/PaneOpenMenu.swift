import Bonsplit
import SwiftUI

struct PaneOpenMenu: View {
  @Bindable var model: AppModel
  let pane: PaneID
  var body: some View {
    IconMenu(title: "開く", image: "plus") {
      Button {
        model.panes.open(.terminal, in: pane)
      } label: {
        Label("Terminal", image: "terminal")
      }
    }
    .disabled(!model.layoutLoaded)
    .padding(.horizontal, UIStyle.spacing)
  }
}
