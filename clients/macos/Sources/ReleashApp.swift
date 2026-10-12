import SwiftUI

@main struct ReleashApp: App {
  @NSApplicationDelegateAdaptor(WorkspaceWindows.self) private var windows
  private var model: AppModel { windows.model }

  var body: some Scene {
    Settings { EmptyView() }
    .commands {
      CommandGroup(replacing: .appSettings) {}
      CommandGroup(replacing: .newItem) {
        Button("New Releash Window") { windows.openWindow() }.keyboardShortcut("n")
      }
      CommandMenu("タブ") {
        Button("タブを閉じる") { model.panes.closeCurrentTab() }
          .keyboardShortcut("w").disabled(!model.layoutLoaded)
        Button("ほかのタブを閉じる") { model.panes.closeOtherTabs() }
          .keyboardShortcut("t", modifiers: [.command, .option]).disabled(!model.layoutLoaded)
        Divider()
        Button("前のタブ") { model.panes.controller.selectPreviousTab() }
          .keyboardShortcut(.leftArrow, modifiers: [.command, .option]).disabled(
            !model.layoutLoaded)
        Button("次のタブ") { model.panes.controller.selectNextTab() }
          .keyboardShortcut(.rightArrow, modifiers: [.command, .option]).disabled(
            !model.layoutLoaded)
      }
    }
  }
}

struct RootView: View {
  @Bindable var model: AppModel
  @State private var creating = false
  @State private var sidebarWidth = UIStyle.sidebarWidth
  @State private var rightPanelWidth = UIStyle.sidebarWidth
  @State private var rightPanelExpanded = true
  @State private var workspaceWidth: CGFloat = 0
  @State private var hoveringDivider = false
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
                maxWidth: UIStyle.sidebarMaximumWidth, maxHeight: .infinity
              )
              .onGeometryChange(for: CGFloat.self) {
                $0.size.width
              } action: {
                sidebarWidth = $0
              }

            HSplitView {
              Group {
                if model.selectedWorktree != nil {
                  if model.layoutLoaded {
                    WorkspacePaneView(model: model)
                  } else {
                    ProgressView("配置を読み込んでいます")
                      .frame(maxWidth: .infinity, maxHeight: .infinity)
                  }
                } else {
                  VStack(spacing: UIStyle.spacing) {
                    Text("Worktreeを選択").font(.headline)
                    Text("サイドバーから作業するWorktreeを選んでください")
                      .font(.callout).foregroundStyle(.secondary)
                  }
                  .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
              }
              .frame(maxWidth: .infinity, maxHeight: .infinity)
              .layoutPriority(1)
              RightPanelView(expanded: $rightPanelExpanded)
                .onGeometryChange(for: CGFloat.self) {
                  $0.size.width
                } action: {
                  rightPanelWidth = $0
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .layoutPriority(1)
            .overlay(alignment: .trailing) {
              Rectangle().fill(Color(nsColor: .separatorColor))
                .background(Color(nsColor: .controlBackgroundColor))
                .frame(width: UIStyle.dividerWidth)
                .offset(x: -rightPanelWidth).allowsHitTesting(false)
            }
          }
          .frame(maxWidth: .infinity, maxHeight: .infinity)
          .overlay(alignment: .leading) {
            Rectangle().fill(Color(nsColor: .separatorColor))
              .background(Color(nsColor: .controlBackgroundColor))
              .frame(width: UIStyle.dividerWidth)
              .offset(x: sidebarWidth).allowsHitTesting(false)
          }
          .onGeometryChange(for: CGFloat.self) { $0.size.width } action: {
            workspaceWidth = $0
          }
          .onContinuousHover { phase in
            switch phase {
            case .active(let location):
              hoveringDivider =
                abs(location.x - sidebarWidth) <= UIStyle.dividerCursorWidth / 2
                || (rightPanelExpanded
                  && abs(location.x - (workspaceWidth - rightPanelWidth))
                    <= UIStyle.dividerCursorWidth / 2)
            case .ended:
              hoveringDivider = false
            }
          }
          .pointerStyle(hoveringDivider ? .columnResize : nil)
          Divider()
          Color(nsColor: .controlBackgroundColor).frame(height: UIStyle.footerHeight)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(isPresented: $creating) { CreateWorktreeView(model: model) }
      }
    }
    .frame(
      minWidth: UIStyle.windowMinimumWidth, maxWidth: .infinity,
      minHeight: UIStyle.windowMinimumHeight, maxHeight: .infinity
    )
    .overlay(alignment: .topLeading) {
      HStack(spacing: UIStyle.spacing) {
        WindowControls().frame(width: UIStyle.windowControlsWidth)
        Color.clear.contentShape(Rectangle()).gesture(WindowDragGesture())
          .pointerStyle(.grabIdle)
      }
      .padding(.horizontal, UIStyle.padding)
      .frame(height: UIStyle.tabBarHeight)
      .frame(width: UIStyle.sidebarWidth)
    }
    .background(Color(nsColor: .controlBackgroundColor))
    .clipShape(RoundedRectangle(cornerRadius: UIStyle.contentCornerRadius))
    .padding(UIStyle.windowPadding)
    .background(
      Color(nsColor: .underPageBackgroundColor),
      in: RoundedRectangle(cornerRadius: UIStyle.windowCornerRadius)
    )
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

struct WindowControls: NSViewRepresentable {
  func makeNSView(context: Context) -> ControlsView { ControlsView() }
  func updateNSView(_ nsView: ControlsView, context: Context) {}

  final class ControlsView: NSView {
    override init(frame: NSRect) {
      super.init(frame: frame)
      let actions: [(NSWindow.ButtonType, Selector, String)] = [
        (.closeButton, #selector(NSWindow.close), "閉じる"),
        (.miniaturizeButton, #selector(NSWindow.miniaturize(_:)), "しまう"),
        (.zoomButton, #selector(NSWindow.toggleFullScreen(_:)), "フルスクリーン"),
      ]
      for entry in actions {
        guard let button = NSWindow.standardWindowButton(entry.0, for: .titled) else { continue }
        button.action = entry.1
        button.setAccessibilityLabel(entry.2)
        addSubview(button)
      }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      window?.setAccessibilitySubrole(.standardWindow)
      if let window { NSApp.addWindowsItem(window, title: "Releash", filename: false) }
      for case let button as NSButton in subviews { button.target = window }
    }
    override func layout() {
      super.layout()
      for (index, button) in subviews.enumerated() {
        button.setFrameOrigin(
          NSPoint(
            x: CGFloat(index) * UIStyle.windowControlSpacing,
            y: (bounds.height - button.frame.height) / 2))
      }
    }
    override var mouseDownCanMoveWindow: Bool { false }
  }
}
