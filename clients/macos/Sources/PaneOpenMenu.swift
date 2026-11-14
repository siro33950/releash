import AppKit
import Bonsplit
import SwiftUI

struct PaneOpenMenu: View {
  @Bindable var model: AppModel
  let pane: PaneID
  var body: some View {
    Group {
      if let path = model.selectedWorktree {
        SessionWorkflowMenu(model: model, path: path, pane: pane)
      } else {
        IconMenu(title: "開く", image: "plus") {
          Button {
            model.panes.open(.terminal, in: pane)
          } label: {
            Label("Terminal", image: "terminal")
          }
        }
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
  @State private var anchor = NSView()

  var body: some View {
    Button {
      menu().popUp(positioning: nil, at: NSPoint(x: 0, y: anchor.bounds.minY), in: anchor)
    } label: {
      CardIcon(image: Image("plus"))
    }
    .buttonStyle(IconButtonStyle())
    .iconButtonFrame()
    .background(MenuAnchor(view: anchor))
    .help(pane == nil ? "開始" : "開く")
    .accessibilityLabel(pane == nil ? "開始" : "開く")
  }

  private func menu() -> NSMenu {
    let menu = NSMenu()
    menu.autoenablesItems = false
    let sessions = NSMenu()
    for provider in model.providers {
      sessions.addItem(
        ActionItem(provider.displayName) {
          model.perform {
            try await model.startSession(provider: provider.wireName, path: path, pane: pane)
          }
        })
    }
    menu.addItem(withTitle: "Session", action: nil, keyEquivalent: "").submenu = sessions
    let workflows = NSMenu()
    for workflow in model.workflows {
      workflows.addItem(
        ActionItem(workflow.name) {
          model.perform { try await model.startWorkflow(name: workflow.name, path: path) }
        })
    }
    let workflow = menu.addItem(withTitle: "Workflow", action: nil, keyEquivalent: "")
    workflow.submenu = workflows
    workflow.isEnabled = model.pendingRoots[path] == nil
    if let pane {
      let terminal = ActionItem("Terminal") { model.panes.open(.terminal, in: pane) }
      terminal.image = NSImage(named: "terminal")
      menu.addItem(terminal)
    }
    return menu
  }

  private struct MenuAnchor: NSViewRepresentable {
    let view: NSView
    func makeNSView(context: Context) -> NSView { view }
    func updateNSView(_ nsView: NSView, context: Context) {}
  }

  private final class ActionItem: NSMenuItem {
    private let perform: () -> Void
    init(_ title: String, perform: @escaping () -> Void) {
      self.perform = perform
      super.init(title: title, action: #selector(invoke), keyEquivalent: "")
      target = self
    }
    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("ActionItem requires an action") }
    @objc private func invoke() { perform() }
  }
}
