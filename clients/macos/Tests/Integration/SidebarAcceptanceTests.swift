import AppKit
import ApplicationServices
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SidebarAcceptanceTests: XCTestCase {
  @MainActor private func repository(_ names: [String]) -> Releash_Client_V1_WorkspaceRepositoryList
  {
    .with {
      $0.path = "/acceptance"
      $0.branches.items = names.map { name in
        .with {
          $0.name = name
          $0.worktreePath = "/acceptance/" + name
        }
      }
      $0.worktrees.items = names.map { name in .with { $0.path = "/acceptance/" + name } }
    }
  }

  @MainActor func test_サイドバー_B001_B002_B018_上端の作成と下端の追加更新を右寄せする() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.repositories = [repository(["main"])] }
    let model = try await testModel(http)
    // When
    let host = await ViewHarness(
      SidebarView(model: model, creating: .constant(false)),
      size: NSSize(width: 340, height: 700))
    defer { host.close() }
    try await eventuallyView(host) { host.element("Worktreeを作成") != nil }
    // Then
    XCTAssertNil(host.element("Workspaces"))
    let create = try frame("Worktreeを作成", in: host)
    let add = try frame("Repositoryを追加", in: host)
    let refresh = try frame("更新", in: host)
    let bounds = try XCTUnwrap(host.window.contentView).bounds
    XCTAssertLessThan(create.midY, 40)
    XCTAssertGreaterThan(create.midX, bounds.maxX - 50)
    XCTAssertGreaterThan(add.midY, bounds.maxY - 50)
    XCTAssertEqual(add.midY, refresh.midY, accuracy: 1)
    XCTAssertGreaterThan(refresh.midX, add.midX)
    XCTAssertGreaterThan(add.midX, bounds.midX)
    XCTAssertGreaterThan(refresh.midX, bounds.maxX - 50)
    let buttons = try axElements(in: host).filter {
      attribute($0, kAXRoleAttribute) as? String == kAXButtonRole
    }
    let buttonFrames = try buttons.map { try frame($0, in: host) }
    XCTAssertEqual(buttonFrames.filter { $0.midY >= 0 && $0.midY < 40 }.count, 1)
    XCTAssertEqual(buttonFrames.filter { $0.midY > bounds.maxY - 50 }.count, 2)
    for (label, icon) in [
      ("Worktreeを作成", "plus"), ("Repositoryを追加", "folder-plus"), ("更新", "refresh-cw"),
    ] {
      try await assertIcon(icon, in: host, element: label)
    }
    let root = await ViewHarness(RootView(model: model))
    defer { root.close() }
    try await eventuallyView(root) { root.element("Worktreeを作成") != nil }
    XCTAssertEqual(
      try frame("Worktreeを作成", in: root).midY,
      try frame("閉じる", in: root).midY, accuracy: 1)
    await model.stop()
  }

  @MainActor func test_サイドバー_B003_更新操作後に購読した一覧が表示される() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.repositories = [repository(["before"])] }
    let model = try await testModel(http)
    let host = await ViewHarness(SidebarView(model: model, creating: .constant(false)))
    defer { host.close() }
    try await eventuallyView(host) { host.element("worktree:before") != nil }
    // When
    try await host.press("更新")
    try await eventually { http.requests("RefreshWorkspaces").count == 1 }
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [repository(["after"])]
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) {
      host.element("worktree:after") != nil && host.element("worktree:before") == nil
    }
    await model.stop()
  }

  @MainActor func test_サイドバー_B015_B016_B017_B019_B020_選択したworktreeの変更数だけをバッジに出す() async throws {
    // Given
    let http = TestHTTPClient()
    var repo = repository(["three", "hundred", "zero", "failed"])
    for (index, count) in [3, 100, 0].enumerated() {
      repo.branches.items[index].dirtyCount = UInt64(count)
    }
    repo.branches.items[3].dirtyCountError = "Count failed"
    http.state.withLock { $0.repositories = [repo] }
    let model = try await testModel(http)
    let host = await ViewHarness(
      HStack(spacing: 0) {
        SidebarView(model: model, creating: .constant(false)).frame(width: 340)
        RightPanelView(expanded: .constant(true), model: model)
      })
    defer { host.close() }
    try await eventuallyView(host) { host.element("worktree:three") != nil }
    for (selection, expected) in [
      ("", ""), ("three", "3"), ("hundred", "99+"), ("zero", ""), ("failed", ""),
    ] {
      // When
      if !selection.isEmpty { try await host.press("worktree:" + selection) }
      // Then
      try await eventuallyView(host) { host.element("right-panel:sourceControl") != nil }
      if expected.isEmpty {
        let toggle = try frame("right-panel:sourceControl", in: host)
        try await eventuallyView(host) {
          (try? hasGrayBadge(in: host, rect: toggle.insetBy(dx: -10, dy: -10))) == false
        }
        XCTAssertFalse(try hasGrayBadge(in: host, rect: toggle.insetBy(dx: -10, dy: -10)))
      } else {
        try await eventuallyView(host) { host.element(expected) != nil }
        let badge = try frame(expected, in: host)
        let toggle = try frame("right-panel:sourceControl", in: host)
        XCTAssertGreaterThan(badge.midX, toggle.midX)
        XCTAssertLessThan(badge.midY, toggle.midY)
        XCTAssertTrue(try hasGrayBadge(in: host, rect: badge.insetBy(dx: -12, dy: -12)))
      }
    }
    await model.stop()
  }
  @MainActor private func axElements(in host: ViewHarness) throws -> [AXUIElement] {
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
    let window = try XCTUnwrap(
      windows.first { attribute($0, kAXTitleAttribute) as? String == host.window.title })
    var visited: [AXUIElement] = []
    func collect(_ item: AXUIElement) {
      guard !visited.contains(where: { CFEqual($0, item) }) else { return }
      visited.append(item)
      for child in attribute(item, kAXChildrenAttribute) as? [AXUIElement] ?? [] { collect(child) }
    }
    collect(window)
    return visited
  }

  private func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success else {
      return nil
    }
    return value
  }

  @MainActor private func raster(in host: ViewHarness, rect: NSRect) throws -> SidebarIconRaster {
    host.window.appearance = NSAppearance(named: .aqua)
    let view = try XCTUnwrap(host.window.contentView)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let region = rect.integral.intersection(view.bounds)
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: region))
    view.cacheDisplay(in: region, to: bitmap)
    return try SidebarIconRaster(bitmap: bitmap)
  }

  @MainActor private func hasGrayBadge(in host: ViewHarness, rect: NSRect) throws -> Bool {
    let actual = try raster(in: host, rect: rect)
    for diameter in 12...28 {
      let side = CGFloat(diameter)
      let view = NSHostingView(
        rootView: Circle().fill(.black)
          .frame(width: side, height: side).padding(2).background(.white))
      view.setFrameSize(view.fittingSize)
      view.layoutSubtreeIfNeeded()
      let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
      view.cacheDisplay(in: view.bounds, to: bitmap)
      if actual.find(try SidebarIconRaster(bitmap: bitmap), color: .gray, ignoringCenter: true)
        != nil
      {
        return true
      }
    }
    return false
  }

  @MainActor private static func reference(_ name: String, size: Int, scale: CGFloat? = nil)
    throws
    -> SidebarIconRaster
  {
    let svg = try XCTUnwrap(LucideAcceptanceReference.svg[name])
    let image = try XCTUnwrap(NSImage(data: Data(svg.utf8)))
    return try SidebarIconRaster(image: image, size: size, scale: scale)
  }

  @MainActor private func frame(_ name: String, in host: ViewHarness) throws -> NSRect {
    let keys = [
      kAXIdentifierAttribute, kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute,
    ]
    let element = try XCTUnwrap(
      try axElements(in: host).first { element in
        keys.contains { attribute(element, $0) as? String == name }
      }, name)
    return try frame(element, in: host)
  }

  @MainActor private func frame(_ element: AXUIElement, in host: ViewHarness) throws -> NSRect {
    let value = try XCTUnwrap(attribute(element, "AXFrame"))
    var screen = CGRect.zero
    XCTAssertTrue(AXValueGetValue(value as! AXValue, .cgRect, &screen))
    screen.origin.y = try XCTUnwrap(NSScreen.screens.first).frame.height - screen.maxY
    let view = try XCTUnwrap(host.window.contentView)
    return view.convert(host.window.convertFromScreen(screen), from: nil)
  }

  @discardableResult @MainActor private func assertIcon(
    _ name: String?, in host: ViewHarness, element: String? = nil, rect: NSRect? = nil,
    sizes: [Int] = Array(10...20), color: Color? = nil,
    file: StaticString = #filePath, line: UInt = #line
  ) async throws -> NSRect {
    if let element { try await eventuallyView(host, { host.element(element) != nil }) }
    let view = try XCTUnwrap(host.window.contentView)
    host.window.appearance = NSAppearance(named: .aqua)
    view.layoutSubtreeIfNeeded()
    host.window.displayIfNeeded()
    let bounds = try rect ?? element.map { try frame($0, in: host) } ?? view.bounds
    // Settings のタブはタイトルバーに配置され、contentView の外側にある。
    let captureView = try view.bounds.contains(bounds) ? view : XCTUnwrap(view.superview)
    let region = captureView.convert(bounds, from: view).integral.intersection(captureView.bounds)
    let bitmap = try XCTUnwrap(
      captureView.bitmapImageRepForCachingDisplay(in: region),
      "\(element ?? name ?? "Lucide icon"): bounds=\(bounds), capture=\(captureView.bounds), region=\(region)",
      file: file, line: line)
    captureView.cacheDisplay(in: region, to: bitmap)
    let actual = try SidebarIconRaster(bitmap: bitmap)
    var match: NSRect?
    for size in sizes {
      for candidate in name.map({ [$0] }) ?? LucideAcceptanceReference.svg.keys.sorted() {
        let expected = try Self.reference(
          candidate, size: size, scale: CGFloat(bitmap.pixelsWide) / region.width)
        if let found = actual.find(expected, color: color) {
          match = found
          break
        }
      }
      if match != nil { break }
    }
    if match == nil {
      let image = NSImage(size: bitmap.size)
      image.addRepresentation(bitmap)
      let attachment = XCTAttachment(image: image)
      attachment.name = "\(name ?? "Lucide icon")-\(element ?? "view")"
      attachment.lifetime = .keepAlways
      add(attachment)
    }
    let found = try XCTUnwrap(
      match, "\(name ?? "Lucide icon"), \(element ?? "view"), sizes=\(sizes)", file: file,
      line: line)
    let scale = CGFloat(bitmap.pixelsWide) / region.width
    return view.convert(
      NSRect(
        x: region.minX + found.minX / scale, y: region.minY + found.minY / scale,
        width: found.width / scale, height: found.height / scale), from: captureView)
  }

}

private struct SidebarIconRaster {
  let width: Int
  let height: Int
  let pixels: [SIMD3<Double>]

  init(width: Int, height: Int, pixels: [SIMD3<Double>]) {
    self.width = width
    self.height = height
    self.pixels = pixels
  }

  /// 参照画像を、実画面と同じ描画経路（NSHostingView を cacheDisplay で写す）で作る。
  /// 描画経路が違うと、8 ポイントのような小さい大きさでは同じ SVG でも画素が合わない
  @MainActor init(image: NSImage, size: Int, scale: CGFloat? = nil) throws {
    let side = CGFloat(size)
    let view = NSHostingView(
      rootView: Image(nsImage: image).resizable().renderingMode(.template)
        .foregroundStyle(.black).frame(width: side, height: side).background(.white))
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: side, height: side), styleMask: [.borderless],
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    window.contentView = view
    view.layoutSubtreeIfNeeded()
    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
    view.cacheDisplay(in: view.bounds, to: bitmap)
    if let scale {
      XCTAssertEqual(CGFloat(bitmap.pixelsWide), side * scale, "参照画像と実画面の倍率が違う")
    }
    try self.init(bitmap: bitmap)
  }

  init(bitmap: NSBitmapImageRep) throws {
    // 画面を写した画像は表示先のディスプレイの色空間である。色を比べる前に sRGB に直す
    let bitmap = try XCTUnwrap(bitmap.converting(to: .sRGB, renderingIntent: .default))
    let width = bitmap.pixelsWide
    let height = bitmap.pixelsHigh
    self.width = width
    self.height = height
    pixels = try (0..<height).flatMap { y in
      try (0..<width).map { x in
        let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB))
        let alpha = Double(color.alphaComponent)
        return SIMD3(
          Double(color.redComponent), Double(color.greenComponent), Double(color.blueComponent))
          * alpha + (1 - alpha)
      }
    }
  }

  @MainActor func find(_ reference: Self, color: Color?, ignoringCenter: Bool = false) -> NSRect? {
    guard width >= reference.width, height >= reference.height else { return nil }
    let weights = reference.pixels.map { 1 - ($0.x + $0.y + $0.z) / 3 }
    let energy = weights.reduce(0) { $0 + $1 * $1 }
    guard energy > 1 else { return nil }
    let required = color.flatMap { NSColor($0).usingColorSpace(.sRGB) }.map {
      SIMD3(Double($0.redComponent), Double($0.greenComponent), Double($0.blueComponent))
    }
    func ignored(_ index: Int) -> Bool {
      let dx = Double(index % reference.width) - Double(reference.width) / 2
      let dy = Double(index / reference.width) - Double(reference.height) / 2
      return ignoringCenter
        && (weights[index] < 0.01
          || dx * dx + dy * dy < pow(Double(reference.width) * 0.3, 2))
    }
    let samples = weights.indices.filter { weights[$0] > 0.5 && !ignored($0) }
    let sampleEnergy = samples.reduce(0) { $0 + weights[$1] * weights[$1] }
    for y in 0...(height - reference.height) {
      for x in 0...(width - reference.width) {
        let background = pixels[y * width + x]
        var contrast = SIMD3<Double>.zero
        for index in samples {
          contrast +=
            (pixels[(y + index / reference.width) * width + x + index % reference.width]
              - background) * weights[index]
        }
        contrast /= sampleEnergy
        let foreground = background + contrast
        let magnitude = (contrast * contrast).sum()
        guard magnitude > 0.12 else { continue }
        if let required, ((foreground - required) * (foreground - required)).sum() > 0.12 {
          continue
        }
        var error = 0.0
        let limit = energy * magnitude * 0.10
        for index in weights.indices where !ignored(index) {
          let actual = pixels[(y + index / reference.width) * width + x + index % reference.width]
          let delta = actual - (background + contrast * weights[index])
          error += (delta * delta).sum()
          if error > limit { break }
        }
        if error <= limit {
          return NSRect(x: x, y: y, width: reference.width, height: reference.height)
        }
      }
    }
    return nil
  }

}
