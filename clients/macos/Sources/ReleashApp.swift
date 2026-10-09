import SwiftUI

@main struct ReleashApp: App {
  @State private var model = AppModel()
  var body: some Scene {
    WindowGroup {
      RootView(model: model).task {
        if ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil {
          await model.start()
        }
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
            SidebarView(model: model, creating: $creating).frame(minWidth: UIStyle.sidebarWidth)
            if model.selectedWorktree != nil {
              PaneView(layout: model.layout, model: model).disabled(!model.layoutLoaded)
            } else {
              ContentUnavailableView("Worktreeを選択", systemImage: "square.split.2x2")
            }
          }
          Divider()
          Color.clear.frame(height: UIStyle.footerHeight)
        }
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
