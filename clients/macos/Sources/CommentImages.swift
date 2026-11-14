import SwiftUI
import Textual
import WebKit

extension AppModel {
  func commentImage(_ url: URL) async throws -> Releash_Client_V1_CommentImageSnapshot {
    let (stream, continuation) = AsyncThrowingStream<Releash_Client_V1_CommentImageSnapshot, Error>
      .makeStream(bufferingPolicy: .bufferingNewest(1))
    let id = subscribe(
      "comment-image", [url.absoluteString],
      onFailure: {
        continuation.finish(throwing: $0)
      },
      onValue: { payload in
        guard case .commentImage(let image) = payload.value else { return }
        continuation.yield(image)
        continuation.finish()
      })
    defer { unsubscribe(id) }
    for try await image in stream { return image }
    throw CancellationError()
  }
}

struct CommentImageAttachmentLoader: AttachmentLoader {
  let model: AppModel

  func attachment(for url: URL, text: String, environment: ColorEnvironmentValues) async throws
    -> some Attachment
  {
    let value = try await model.commentImage(url)
    guard let image = NSImage(data: value.data), image.size.width > 0, image.size.height > 0 else {
      throw URLError(.cannotDecodeContentData)
    }
    return CommentImageAttachment(data: value.data, size: image.size, description: text)
  }
}

private struct CommentImageAttachment: Attachment {
  let data: Data
  let size: CGSize
  let description: String

  var body: some View {
    if let image = NSImage(data: data) {
      Image(nsImage: image).resizable().scaledToFit().accessibilityLabel(description)
    }
  }

  func sizeThatFits(_ proposal: ProposedViewSize, in environment: TextEnvironmentValues) -> CGSize {
    let width = min(proposal.width ?? size.width, size.width)
    return CGSize(width: width, height: width * size.height / size.width)
  }
}

@MainActor final class CommentImageSchemeHandler: NSObject, WKURLSchemeHandler {
  let model: AppModel
  private var tasks: [ObjectIdentifier: Task<Void, Never>] = [:]

  init(model: AppModel) { self.model = model }

  func webView(_ webView: WKWebView, start urlSchemeTask: any WKURLSchemeTask) {
    let id = ObjectIdentifier(urlSchemeTask)
    tasks[id] = Task {
      defer { tasks.removeValue(forKey: id) }
      do {
        guard let requestURL = urlSchemeTask.request.url,
          let raw = URLComponents(url: requestURL, resolvingAgainstBaseURL: false)?.queryItems?
            .first(where: { $0.name == "url" })?.value,
          let url = URL(string: raw)
        else { throw URLError(.badURL) }
        let value = try await model.commentImage(url)
        try Task.checkCancellation()
        urlSchemeTask.didReceive(
          URLResponse(
            url: requestURL, mimeType: value.mediaType, expectedContentLength: value.data.count,
            textEncodingName: nil))
        urlSchemeTask.didReceive(value.data)
        urlSchemeTask.didFinish()
      } catch {
        if !Task.isCancelled { urlSchemeTask.didFailWithError(error) }
      }
    }
  }

  func webView(_ webView: WKWebView, stop urlSchemeTask: any WKURLSchemeTask) {
    tasks.removeValue(forKey: ObjectIdentifier(urlSchemeTask))?.cancel()
  }
}
