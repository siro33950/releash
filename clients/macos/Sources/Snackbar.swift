import SwiftUI

extension View {
  func snackbar(_ model: AppModel) -> some View {
    overlay(alignment: .bottom) {
      VStack(spacing: UIStyle.rowSpacing) {
        ForEach(model.notices.suffix(UIStyle.snackbarLimit)) { notice in
          SnackbarItem(model: model, notice: notice)
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
      }
      .padding(.bottom, UIStyle.footerHeight + UIStyle.padding)
      .animation(.default, value: model.notices)
    }
  }
}

private struct SnackbarItem: View {
  let model: AppModel
  let notice: AppModel.Notice

  var body: some View {
    ViewThatFits(in: .vertical) {
      message
      ScrollView { message }
    }
    .frame(
      maxWidth: UIStyle.snackbarMaximumWidth, maxHeight: UIStyle.snackbarMaximumHeight,
      alignment: .leading
    )
    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: UIStyle.contentCornerRadius))
    .shadow(radius: UIStyle.snackbarShadowRadius)
    .onHover { hovering in
      if hovering {
        model.hoveredNotice = notice.id
      } else if model.hoveredNotice == notice.id {
        model.hoveredNotice = nil
      }
    }
    .accessibilityIdentifier("snackbar")
  }

  private var message: some View {
    Text(notice.message)
      .textSelection(.enabled)
      .padding(UIStyle.padding)
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}
