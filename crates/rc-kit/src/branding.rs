//! One bundled mark for native windows, the project menu and About.
use egui::{Color32, ColorImage, Id, Rect, TextureHandle, TextureOptions, Ui, pos2};

pub const ICON_PNG: &[u8] = include_bytes!("../../../assets/icon.png");

pub fn icon_data() -> egui::IconData {
    // The asset is generated and validated in the repository, never supplied by a user.
    eframe::icon_data::from_png_bytes(ICON_PNG).expect("bundled DotCraft icon is a valid PNG")
}

pub fn paint(ui: &Ui, rect: Rect) {
    let (size, png): (u16, &[u8]) = if rect.width() * ui.ctx().pixels_per_point() <= 32.0 {
        (32, include_bytes!("../../../assets/icon-32.png"))
    } else {
        (64, include_bytes!("../../../assets/icon-64.png"))
    };
    let id = Id::new(("dotcraft_application_icon", size));
    let texture = ui
        .ctx()
        .data_mut(|data| data.get_temp::<TextureHandle>(id))
        .unwrap_or_else(|| {
            let icon = eframe::icon_data::from_png_bytes(png)
                .expect("bundled DotCraft UI icon is a valid PNG");
            let image = ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            let texture = ui
                .ctx()
                .load_texture("DotCraft", image, TextureOptions::LINEAR);
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_and_in_app_icon_decodes_with_transparent_padding() {
        let icon = super::icon_data();
        assert_eq!((icon.width, icon.height), (512, 512));
        assert_eq!(icon.rgba.len(), 512 * 512 * 4);
        assert_eq!(icon.rgba[3], 0);
        assert!(icon.rgba[(256 * 512 + 256) * 4 + 3] >= 250);
    }
}
