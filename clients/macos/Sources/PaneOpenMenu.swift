import Bonsplit
import SwiftUI

struct PaneOpenMenu: View {
  @Bindable var model: AppModel
  let pane: PaneID
  var body: some View {
    IconMenu(title: "開く", image: "plus") {
      if let path = model.selectedWorktree {
        SessionWorkflowMenu(
          model: model, path: path, pane: pane, workflowPending: model.pendingRoots[path] != nil)
      }
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

struct SessionWorkflowMenu: View {
  @Bindable var model: AppModel
  let path: String
  var pane: PaneID? = nil
  let workflowPending: Bool

  var body: some View {
    Menu("Session") {
      ForEach(model.providers, id: \.value) { provider in
        Button(provider.displayName) {
          model.perform {
            try await model.startSession(provider: provider.wireName, path: path, pane: pane)
          }
        }
      }
    }
    if workflowPending {
      Button("Workflow", action: {}).disabled(true)
    } else {
      Menu("Workflow") {
        ForEach(model.workflows, id: \.name) { workflow in
          Button(workflow.name) {
            model.perform { try await model.startWorkflow(name: workflow.name, path: path) }
          }
        }
      }
    }
  }
}
