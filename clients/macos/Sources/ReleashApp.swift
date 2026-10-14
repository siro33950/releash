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
  @State private var rightPanelWidth = UIStyle.centerInitialWidth
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
          WorkspaceColumnSplit(
            minimumLeadingWidth: UIStyle.sidebarWidth, maximumLeadingWidth: UIStyle.sidebarMaximumWidth,
            minimumTrailingWidth: UIStyle.centerMinimumWidth + (rightPanelExpanded ? UIStyle.rightPanelMinimumWidth : UIStyle.collapsedPanelWidth),
            initialLeadingWidth: UIStyle.sidebarWidth
          ) {
            SidebarView(model: model, creating: $creating)
              .frame(maxHeight: .infinity)
              .onGeometryChange(for: CGFloat.self) {
                $0.size.width
              } action: {
                sidebarWidth = $0
              }
          } trailing: {
            WorkspaceColumnSplit(
              minimumLeadingWidth: UIStyle.centerMinimumWidth,
              minimumTrailingWidth: rightPanelExpanded ? UIStyle.rightPanelMinimumWidth : UIStyle.collapsedPanelWidth,
              fixedTrailingWidth: rightPanelExpanded ? nil : UIStyle.collapsedPanelWidth
            ) {
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
            } trailing: {
              RightPanelView(expanded: $rightPanelExpanded, model: model)
                .layoutPriority(1)
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
          FooterView(model: model)
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

private struct WorkspaceColumnSplit<Leading: View, Trailing: View>: NSViewRepresentable {
  let minimumLeadingWidth: CGFloat
  var maximumLeadingWidth: CGFloat = .infinity
  let minimumTrailingWidth: CGFloat
  var initialLeadingWidth: CGFloat? = nil
  var fixedTrailingWidth: CGFloat? = nil
  @ViewBuilder let leading: () -> Leading
  @ViewBuilder let trailing: () -> Trailing

  func makeCoordinator() -> Coordinator { Coordinator(self) }
  func makeNSView(context: Context) -> SplitView {
    let split = SplitView()
    split.isVertical = true; split.dividerStyle = .thin; split.delegate = context.coordinator
    let leadingView = NSHostingView(rootView: leading())
    let trailingView = NSHostingView(rootView: trailing())
    leadingView.sizingOptions = []; trailingView.sizingOptions = []
    split.addArrangedSubview(leadingView); split.addArrangedSubview(trailingView)
    return split
  }
  func updateNSView(_ split: SplitView, context: Context) {
    let coordinator = context.coordinator
    let previous = coordinator.parent
    coordinator.parent = self
    (split.arrangedSubviews[0] as? NSHostingView<Leading>)?.rootView = leading()
    (split.arrangedSubviews[1] as? NSHostingView<Trailing>)?.rootView = trailing()
    var proposed = split.arrangedSubviews[0].frame.width
    if previous.fixedTrailingWidth != fixedTrailingWidth {
      if fixedTrailingWidth != nil {
        coordinator.expandedTrailingWidth = split.arrangedSubviews[1].frame.width
      } else if let width = coordinator.expandedTrailingWidth {
        proposed = split.bounds.width - width
      }
    }
    if previous.minimumLeadingWidth != minimumLeadingWidth
      || previous.maximumLeadingWidth != maximumLeadingWidth
      || previous.minimumTrailingWidth != minimumTrailingWidth
      || previous.fixedTrailingWidth != fixedTrailingWidth {
      coordinator.layout(split, proposedLeadingWidth: proposed)
    }
  }
  final class SplitView: NSSplitView {
    override var dividerThickness: CGFloat { 0 }
  }
  final class Coordinator: NSObject, NSSplitViewDelegate {
    var parent: WorkspaceColumnSplit
    var initialized = false
    var expandedTrailingWidth: CGFloat?
    init(_ parent: WorkspaceColumnSplit) { self.parent = parent }

    func splitView(_ splitView: NSSplitView, constrainMinCoordinate proposedMinimumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
      if let width = parent.fixedTrailingWidth { return splitView.bounds.width - width }
      return max(proposedMinimumPosition, parent.minimumLeadingWidth)
    }
    func splitView(_ splitView: NSSplitView, constrainMaxCoordinate proposedMaximumPosition: CGFloat, ofSubviewAt dividerIndex: Int) -> CGFloat {
      min(proposedMaximumPosition, parent.maximumLeadingWidth, splitView.bounds.width - parent.minimumTrailingWidth)
    }
    func splitView(_ splitView: NSSplitView, resizeSubviewsWithOldSize oldSize: NSSize) {
      guard splitView.arrangedSubviews.count == 2 else { return }
      let proposed = initialized ? splitView.arrangedSubviews[0].frame.width : (parent.initialLeadingWidth ?? splitView.bounds.width / 2)
      layout(splitView, proposedLeadingWidth: proposed)
    }
    func layout(_ splitView: NSSplitView, proposedLeadingWidth proposed: CGFloat) {
      guard splitView.arrangedSubviews.count == 2, splitView.bounds.width > 0 else { return }
      let width = splitView.bounds.width
      let leadingWidth = parent.fixedTrailingWidth.map { width - $0 }
        ?? max(parent.minimumLeadingWidth, min(proposed, parent.maximumLeadingWidth, width - parent.minimumTrailingWidth))
      splitView.arrangedSubviews[0].frame = NSRect(x: 0, y: 0, width: leadingWidth, height: splitView.bounds.height)
      splitView.arrangedSubviews[1].frame = NSRect(x: leadingWidth, y: 0, width: max(0, width - leadingWidth), height: splitView.bounds.height)
      initialized = width >= parent.minimumLeadingWidth + parent.minimumTrailingWidth
    }
    func splitView(_ splitView: NSSplitView, effectiveRect proposedEffectiveRect: NSRect, forDrawnRect drawnRect: NSRect, ofDividerAt dividerIndex: Int) -> NSRect {
      NSRect(x: drawnRect.minX - UIStyle.dividerCursorWidth / 2, y: 0, width: UIStyle.dividerCursorWidth, height: splitView.bounds.height)
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
