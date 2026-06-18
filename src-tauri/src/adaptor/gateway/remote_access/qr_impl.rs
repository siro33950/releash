use crate::domain::remote_access::QrRenderGateway;

pub struct QrCodeRenderGateway;

impl QrRenderGateway for QrCodeRenderGateway {
    fn generate_qr_svg(&self, data: &str) -> Result<String, String> {
        use qrcode::render::svg;
        use qrcode::QrCode;

        let code = QrCode::new(data).map_err(|e| format!("QRコード生成失敗: {e}"))?;
        let svg = code
            .render::<svg::Color>()
            .min_dimensions(200, 200)
            .dark_color(svg::Color("#e0e0e0"))
            .light_color(svg::Color("#1a1a1a"))
            .build();
        Ok(svg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_qr_svg_valid() {
        let svg = QrCodeRenderGateway
            .generate_qr_svg("http://127.0.0.1:9700?token=test")
            .unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }
}
