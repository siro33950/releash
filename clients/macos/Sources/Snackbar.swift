import SwiftUI

extension View {
  func snackbar(_ model: AppModel) -> some View {
    overlay(alignment: .bottom) {
      VStack(spacing: UIStyle.rowSpacing) {
        ForEach(model.notices.suffix(UIStyle.snackbarLimit)) { notice in
          Text(notice.message)
            .textSelection(.enabled)
            .padding(UIStyle.padding)
            .frame(maxWidth: UIStyle.snackbarMaximumWidth, alignment: .leading)
            .background(
              .regularMaterial, in: RoundedRectangle(cornerRadius: UIStyle.contentCornerRadius)
            )
            .shadow(radius: UIStyle.snackbarShadowRadius)
            .transition(.move(edge: .bottom).combined(with: .opacity))
            .accessibilityIdentifier("snackbar")
        }
      }
      .padding(.bottom, UIStyle.footerHeight + UIStyle.padding)
      .animation(.default, value: model.notices)
    }
  }
}
