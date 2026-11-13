import AppKit
import SwiftUI
import XCTest

struct IconRaster {
  let width: Int
  let height: Int
  let pixels: [SIMD3<Double>]

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

  @MainActor func find(_ reference: Self, color: Color?) -> NSRect? {
    guard width >= reference.width, height >= reference.height else { return nil }
    let weights = reference.pixels.map { 1 - ($0.x + $0.y + $0.z) / 3 }
    let energy = weights.reduce(0) { $0 + $1 * $1 }
    guard energy > 1 else { return nil }
    let required = color.flatMap { NSColor($0).usingColorSpace(.sRGB) }.map {
      SIMD3(Double($0.redComponent), Double($0.greenComponent), Double($0.blueComponent))
    }
    let samples = weights.indices.filter { weights[$0] > 0.5 }
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
        for index in weights.indices {
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

  func hasColoredPixels(outside rect: NSRect) -> Bool {
    pixels.enumerated().contains { index, pixel in
      !rect.insetBy(dx: -1, dy: -1).contains(NSPoint(x: index % width, y: index / width))
        && max(pixel.x, pixel.y, pixel.z) - min(pixel.x, pixel.y, pixel.z) > 0.3
    }
  }
}
