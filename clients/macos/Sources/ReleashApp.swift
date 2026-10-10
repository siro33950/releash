import SwiftUI

@main struct ReleashApp: App {
  @State private var model = AppModel()
  var body: some Scene {
    WindowGroup {
      RootView(model: model)
        .background(PaneKeyboardEvents(model: model))
        .overlay(alignment: .top) { PaneTabSwitcher(model: model) }
        .task {
          if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil {
            await model.start()
          }
        }
    }
    .commands {
      CommandMenu("タブ") {
        Button("タブを閉じる") {
          if let tab = model.focusedPane?.activeTab { model.closeTabs([tab]) }
        }.keyboardShortcut("w").disabled(model.focusedPane?.activeTab == nil)
        Button("ほかのタブを閉じる") {
          if let pane = model.focusedPane {
            model.closeTabs(pane.tabs.filter { $0.id != pane.activeTab }.map(\.id))
          }
        }.keyboardShortcut("t", modifiers: [.command, .option])
          .disabled((model.focusedPane?.tabs.count ?? 0) < 2)
        Divider()
        Button("前のタブ") { model.cycleTab(-1) }
          .keyboardShortcut(.leftArrow, modifiers: [.command, .option])
        Button("次のタブ") { model.cycleTab(1) }
          .keyboardShortcut(.rightArrow, modifiers: [.command, .option])
      }
    }
  }
}

struct RootView: View {
  @Bindable var model: AppModel
  @State private var creating = false
  var body: some View {
    Group {
      if let reason = model.connectionFailure {
        ContentUnavailableView {
          Label("サーバに接続できません", systemImage: "exclamationmark.triangle")
        } description: {
          Text(reason)
        } actions: {
          Button("再試行") { Task { await model.start() } }
        }
      } else if !model.connected {
        ProgressView("サーバに接続しています")
      } else {
        VStack(spacing: 0) {
          if model.reconnecting {
            Text("再接続しています").frame(maxWidth: .infinity).padding(UIStyle.spacing)
          }
          HSplitView {
            SidebarView(model: model, creating: $creating)
              .frame(
                minWidth: UIStyle.sidebarWidth, idealWidth: UIStyle.sidebarWidth,
                maxWidth: UIStyle.sidebarMaximumWidth, maxHeight: .infinity)
            if model.selectedWorktree != nil {
              PaneView(layout: model.layout, model: model).disabled(!model.layoutLoaded)
                .frame(
                  minWidth: model.layout.minimumLength(along: .horizontal),
                  minHeight: model.layout.minimumLength(along: .vertical))
            } else {
              ContentUnavailableView("Worktreeを選択", systemImage: "square.split.2x2")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
          }
          .frame(maxWidth: .infinity, maxHeight: .infinity)
          Divider()
          Color(nsColor: .controlBackgroundColor).frame(height: UIStyle.footerHeight)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(isPresented: $creating) { CreateWorktreeView(model: model) }
      }
    }
    .alert(
      "操作に失敗しました",
      isPresented: Binding(get: { model.failure != nil }, set: { if !$0 { model.failure = nil } })
    ) {
      Button("閉じる") { model.failure = nil }
    } message: {
      Text(model.failure ?? "")
    }
  }
}
