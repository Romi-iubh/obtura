//! Beautify: draws the frame of [`Frame`] around a finished picture. The live view uses the
//! same pieces ([`backdrop`], [`picture_path`], [`draw_border`]) as the export ([`apply_frame`]).

use crate::{from_surface, to_surface};
use cairo::{Context, Format, ImageSurface, LinearGradient};
use obtura_image::{soften, Bitmap};
use obtura_scene::{Background, Bar, Color, Device, Frame, FrameLayout, Point, RectF};

fn rgba(cr: &Context, c: Color) {
    cr.set_source_rgba(c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0, c.a as f64 / 255.0);
}

/// A rectangle with `top` and `bottom` corner radii, or a torn bottom edge (`torn`, from right
/// to left).
fn rounded(cr: &Context, r: RectF, top: f64, bottom: f64, torn: Option<&[Point]>) {
    let max = (r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (top, bottom) = (top.clamp(0.0, max), bottom.clamp(0.0, max));
    let (x1, y1) = (r.x + r.w, r.y + r.h);
    let q = std::f64::consts::FRAC_PI_2;
    cr.new_sub_path();
    if top > 0.0 {
        cr.arc(r.x + top, r.y + top, top, 2.0 * q, 3.0 * q);
        cr.arc(x1 - top, r.y + top, top, 3.0 * q, 4.0 * q);
    } else {
        cr.move_to(r.x, r.y);
        cr.line_to(x1, r.y);
    }
    match torn {
        Some(pts) => pts.iter().for_each(|p| cr.line_to(p.x, p.y)),
        None if bottom > 0.0 => {
            cr.arc(x1 - bottom, y1 - bottom, bottom, 0.0, q);
            cr.arc(r.x + bottom, y1 - bottom, bottom, q, 2.0 * q);
        }
        None => {
            cr.line_to(x1, y1);
            cr.line_to(r.x, y1);
        }
    }
    cr.close_path();
}

/// The outline of the picture with its bar (corners, torn edge), in result coordinates.
pub fn card_path(cr: &Context, f: &Frame, l: &FrameLayout) {
    let torn = f.is_torn().then(|| Frame::torn_edge(l.card));
    let corner = f.picture_corner(l);
    rounded(cr, l.card, corner, corner, torn.as_deref());
}

/// The outline of the picture alone: what the picture and its marks are clipped to.
pub fn picture_path(cr: &Context, f: &Frame, l: &FrameLayout) {
    let pic = RectF { x: l.pic.x, y: l.pic.y, w: l.card.w, h: l.card.h - l.bar_h };
    let torn = f.is_torn().then(|| Frame::torn_edge(l.card));
    let corner = f.picture_corner(l);
    rounded(cr, pic, if l.bar_h > 0.0 { 0.0 } else { corner }, corner, torn.as_deref());
}

/// What casts the shadow: the device (body, and a laptop's base or a monitor's stand), or the
/// picture with its bar.
fn outline_path(cr: &Context, f: &Frame, l: &FrameLayout) {
    if f.device == Device::None {
        return card_path(cr, f, l);
    }
    let r = f.device.body_corner(l.card.w, l.card.h);
    rounded(cr, l.body, r, r, None);
    for part in device_parts(f.device, l) {
        rounded(cr, part, part.h * 0.25, part.h * 0.25, None);
    }
}

/// A laptop's base, a monitor's neck and foot, below the body.
fn device_parts(device: Device, l: &FrameLayout) -> Vec<RectF> {
    let u = Device::unit(l.card.w, l.card.h);
    let b = l.body;
    let bottom = b.y + b.h;
    match device {
        Device::Laptop => vec![RectF { x: b.x - u * 4.0, y: bottom, w: b.w + u * 8.0, h: (u * 1.3).round() }],
        Device::Monitor => {
            let (neck_w, foot_w) = ((b.w * 0.12).max(u * 2.0), (b.w * 0.36).max(u * 6.0));
            let neck_h = (u * 4.0).round();
            vec![
                RectF { x: b.x + (b.w - neck_w) / 2.0, y: bottom - 1.0, w: neck_w, h: neck_h + 1.0 },
                RectF { x: b.x + (b.w - foot_w) / 2.0, y: bottom + neck_h, w: foot_w, h: (u * 0.9).round() },
            ]
        }
        _ => Vec::new(),
    }
}

/// The device itself: a dark body with a soft edge, its camera, and a laptop's base or a
/// monitor's stand (light grey).
fn draw_device(cr: &Context, f: &Frame, l: &FrameLayout) {
    if f.device == Device::None {
        return;
    }
    let u = Device::unit(l.card.w, l.card.h);
    for part in device_parts(f.device, l) {
        let g = LinearGradient::new(0.0, part.y, 0.0, part.y + part.h);
        g.add_color_stop_rgb(0.0, 0.80, 0.80, 0.82);
        g.add_color_stop_rgb(1.0, 0.58, 0.58, 0.61);
        let _ = cr.set_source(&g);
        rounded(cr, part, (part.h * 0.15).min(4.0), part.h * 0.45, None);
        let _ = cr.fill();
    }
    if f.device == Device::Laptop {
        // The notch to open the lid, in the middle of the base's front.
        let base = device_parts(f.device, l)[0];
        rgba(cr, Color::rgb(0x8a, 0x8a, 0x8f));
        let w = u * 5.0;
        rounded(cr, RectF { x: base.x + (base.w - w) / 2.0, y: base.y, w, h: (base.h * 0.3).max(2.0) }, 0.0, base.h * 0.15, None);
        let _ = cr.fill();
    }
    let r = f.device.body_corner(l.card.w, l.card.h);
    rounded(cr, l.body, r, r, None);
    rgba(cr, Color::rgb(0x1c, 0x1c, 0x1f));
    let _ = cr.fill_preserve();
    rgba(cr, Color::rgb(0x45, 0x45, 0x4b));
    cr.set_line_width((u * 0.15).max(1.0));
    let _ = cr.stroke();
    // The camera: a dot in the middle of the top bezel.
    let top = l.card.y - l.body.y;
    let cam = (u * 0.22).max(2.0);
    rgba(cr, Color::rgb(0x3a, 0x3a, 0x40));
    cr.arc(l.body.x + l.body.w / 2.0, l.body.y + top / 2.0, cam, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

/// Everything behind and around the picture: background, shadow and bar.
pub fn backdrop(f: &Frame, l: &FrameLayout) -> Result<ImageSurface, String> {
    let (w, h) = (l.width as i32, l.height as i32);
    let surface = ImageSurface::create(Format::ARgb32, w.max(1), h.max(1)).map_err(|e| e.to_string())?;
    let cr = Context::new(&surface).map_err(|e| e.to_string())?;
    match f.background {
        Background::None => {}
        Background::Solid(c) => {
            rgba(&cr, c);
            let _ = cr.paint();
        }
        Background::Gradient(a, b) => {
            let g = LinearGradient::new(0.0, 0.0, w as f64, h as f64);
            g.add_color_stop_rgba(0.0, a.r as f64 / 255.0, a.g as f64 / 255.0, a.b as f64 / 255.0, a.a as f64 / 255.0);
            g.add_color_stop_rgba(1.0, b.r as f64 / 255.0, b.g as f64 / 255.0, b.b as f64 / 255.0, b.a as f64 / 255.0);
            let _ = cr.set_source(&g);
            let _ = cr.paint();
        }
        Background::Picture(key) => match obtura_scene::picture(&key) {
            // Covers the whole result: scaled to fill it, centred, the overflow cut.
            Some(b) if b.width > 0 && b.height > 0 => {
                let pic = to_surface(&b)?;
                let k = (w as f64 / b.width as f64).max(h as f64 / b.height as f64);
                let _ = cr.save();
                cr.translate((w as f64 - b.width as f64 * k) / 2.0, (h as f64 - b.height as f64 * k) / 2.0);
                cr.scale(k, k);
                let _ = cr.set_source_surface(&pic, 0.0, 0.0);
                cr.source().set_filter(cairo::Filter::Good);
                let _ = cr.paint();
                let _ = cr.restore();
            }
            // The picture is gone (its file was removed): a plain light background.
            _ => {
                rgba(&cr, Color::rgb(0xec, 0xec, 0xef));
                let _ = cr.paint();
            }
        },
    }
    if f.shadow > 0.0 {
        shadow(&cr, f, l)?;
    }
    draw_device(&cr, f, l);
    if f.bar != Bar::None {
        let _ = cr.save();
        card_path(&cr, f, l);
        cr.clip();
        let (bg, line) = if f.bar == Bar::Dark { (Color::rgb(0x2e, 0x2e, 0x32), Color::rgb(0x1e, 0x1e, 0x20)) } else { (Color::rgb(0xeb, 0xeb, 0xed), Color::rgb(0xd0, 0xd0, 0xd4)) };
        let browser = f.bar == Bar::Browser;
        rgba(&cr, bg);
        cr.rectangle(l.card.x, l.card.y, l.card.w, l.bar_h);
        let _ = cr.fill();
        rgba(&cr, line);
        cr.rectangle(l.card.x, l.card.y + l.bar_h - 1.0, l.card.w, 1.0);
        let _ = cr.fill();
        for (i, c) in [Color::rgb(0xff, 0x5f, 0x57), Color::rgb(0xfe, 0xbc, 0x2e), Color::rgb(0x28, 0xc8, 0x40)].into_iter().enumerate() {
            rgba(&cr, c);
            cr.arc(l.card.x + 18.0 + i as f64 * 20.0, l.card.y + l.bar_h / 2.0, 6.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        if browser && l.card.w > 140.0 {
            // The address field: a white pill across the rest of the bar.
            let (x0, x1) = (l.card.x + 84.0, l.card.x + l.card.w - 16.0);
            let (y, h) = (l.card.y + 9.0, l.bar_h - 18.0);
            let r = h / 2.0;
            cr.new_sub_path();
            cr.arc(x1 - r, y + r, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            cr.arc(x0 + r, y + r, r, std::f64::consts::FRAC_PI_2, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
            rgba(&cr, Color::WHITE);
            let _ = cr.fill_preserve();
            rgba(&cr, line);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        }
        let _ = cr.restore();
    }
    drop(cr);
    Ok(surface)
}

/// A soft shadow under the card, a little lower than it. Made at a quarter of the size (a
/// shadow has no detail) and enlarged.
fn shadow(cr: &Context, f: &Frame, l: &FrameLayout) -> Result<(), String> {
    const K: f64 = 4.0;
    let (sw, sh) = (((l.width as f64) / K).ceil() as i32 + 1, ((l.height as f64) / K).ceil() as i32 + 1);
    let mut small = ImageSurface::create(Format::ARgb32, sw, sh).map_err(|e| e.to_string())?;
    {
        let sc = Context::new(&small).map_err(|e| e.to_string())?;
        sc.scale(1.0 / K, 1.0 / K);
        sc.translate(0.0, f.shadow * 0.3);
        outline_path(&sc, f, l);
        sc.set_source_rgba(0.0, 0.0, 0.0, 0.55);
        let _ = sc.fill();
    }
    let mut bitmap: Bitmap = from_surface(&mut small)?;
    soften(&mut bitmap, ((f.shadow / K) / 2.0).ceil().max(1.0) as u32);
    let soft = to_surface(&bitmap)?;
    let _ = cr.save();
    cr.scale(K, K);
    let _ = cr.set_source_surface(&soft, 0.0, 0.0);
    cr.source().set_filter(cairo::Filter::Good);
    let _ = cr.paint();
    let _ = cr.restore();
    Ok(())
}

/// The line around the picture and its bar.
pub fn draw_border(cr: &Context, f: &Frame, l: &FrameLayout) {
    if f.border <= 0.0 {
        return;
    }
    card_path(cr, f, l);
    rgba(cr, f.border_color);
    cr.set_line_width(f.border);
    cr.set_line_join(cairo::LineJoin::Round);
    let _ = cr.stroke();
}

/// The finished picture inside its frame.
pub fn apply_frame(picture: &Bitmap, f: &Frame) -> Result<Bitmap, String> {
    let l = f.layout(picture.width, picture.height);
    let mut surface = backdrop(f, &l)?;
    {
        let cr = Context::new(&surface).map_err(|e| e.to_string())?;
        let _ = cr.save();
        picture_path(&cr, f, &l);
        cr.clip();
        let pic = to_surface(picture)?;
        let _ = cr.set_source_surface(&pic, l.pic.x, l.pic.y);
        let _ = cr.paint();
        let _ = cr.restore();
        draw_border(&cr, f, &l);
    }
    from_surface(&mut surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUE: [u8; 4] = [0, 0, 255, 255];

    #[test]
    fn the_picture_sits_in_the_padding_on_the_background() {
        let f = Frame { background: Background::Solid(Color::WHITE), padding: 10.0, corner: 0.0, shadow: 0.0, ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(20, 10, BLUE), &f).unwrap();
        assert_eq!((out.width, out.height), (40, 30));
        assert_eq!(out.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(out.pixel(10, 10), BLUE, "first pixel of the picture");
        assert_eq!(out.pixel(29, 19), BLUE, "last pixel of the picture");
        assert_eq!(out.pixel(30, 20), [255, 255, 255, 255]);
    }

    #[test]
    fn rounded_corners_show_the_background_and_a_transparent_one_stays_transparent() {
        let f = Frame { background: Background::None, padding: 4.0, corner: 8.0, shadow: 0.0, ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(40, 40, BLUE), &f).unwrap();
        assert_eq!(out.pixel(4, 4)[3], 0, "the corner is cut off");
        assert_eq!(out.pixel(24, 24), BLUE);
        assert_eq!(out.pixel(0, 20)[3], 0, "no background");
    }

    #[test]
    fn a_shadow_darkens_below_the_picture() {
        let f = Frame { background: Background::Solid(Color::WHITE), padding: 30.0, corner: 0.0, shadow: 20.0, ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(40, 40, BLUE), &f).unwrap();
        let below = out.pixel(50, 74);
        assert!(below[0] < 230, "shadow under the picture: {below:?}");
        assert_eq!(out.pixel(2, 2), [255, 255, 255, 255], "far from it: background");
    }

    #[test]
    fn a_bar_sits_above_the_picture_with_three_buttons() {
        let f = Frame { background: Background::Solid(Color::WHITE), padding: 0.0, corner: 0.0, shadow: 0.0, bar: Bar::Dark, ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(100, 20, BLUE), &f).unwrap();
        assert_eq!(out.height, 54);
        assert_eq!(out.pixel(18, 17), [0xff, 0x5f, 0x57, 255], "the red button");
        assert_eq!(out.pixel(80, 10), [0x2e, 0x2e, 0x32, 255]);
        assert_eq!(out.pixel(50, 40), BLUE);
    }

    #[test]
    fn a_browser_bar_has_an_address_field() {
        let f = Frame { background: Background::Solid(Color::WHITE), padding: 0.0, corner: 0.0, shadow: 0.0, bar: Bar::Browser, ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(300, 20, BLUE), &f).unwrap();
        assert_eq!(out.height, 64);
        assert_eq!(out.pixel(200, 22), [255, 255, 255, 255], "inside the address field");
        assert_eq!(out.pixel(200, 3), [0xeb, 0xeb, 0xed, 255], "the bar around it");
    }

    #[test]
    fn a_torn_edge_cuts_into_the_bottom_of_the_picture() {
        let f = Frame { background: Background::Solid(Color::WHITE), padding: 0.0, corner: 0.0, shadow: 0.0, torn: true, ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(200, 50, BLUE), &f).unwrap();
        let bottom_row_white = (0..200).filter(|&x| out.pixel(x, 48) == [255, 255, 255, 255]).count();
        assert!(bottom_row_white > 40, "{bottom_row_white}");
        assert!((0..200).all(|x| out.pixel(x, 30) == BLUE), "above the teeth the picture is whole");
    }

    #[test]
    fn a_background_picture_covers_the_whole_result() {
        // Left half red, right half green: stretched to cover a wide result, centred.
        let mut bg = Bitmap::filled(20, 20, [255, 0, 0, 255]);
        for y in 0..20 {
            for x in 10..20 {
                bg.set_pixel(x, y, [0, 255, 0, 255]);
            }
        }
        let key = obtura_scene::register_picture(bg);
        let f = Frame { background: Background::Picture(key), padding: 20.0, corner: 0.0, shadow: 0.0, ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(60, 10, BLUE), &f).unwrap();
        assert_eq!((out.width, out.height), (100, 50));
        assert_eq!(out.pixel(2, 2), [255, 0, 0, 255]);
        assert_eq!(out.pixel(97, 47), [0, 255, 0, 255]);
        assert_eq!(out.pixel(50, 25), BLUE);
    }

    #[test]
    fn a_device_is_drawn_around_the_picture() {
        for device in [Device::Phone, Device::Tablet, Device::Laptop, Device::Monitor] {
            let f = Frame { background: Background::Solid(Color::WHITE), padding: 10.0, corner: 0.0, shadow: 0.0, device, ..Frame::default() };
            let out = apply_frame(&Bitmap::filled(300, 200, BLUE), &f).unwrap();
            let l = f.layout(300, 200);
            // The bezel, left of the picture's middle: dark.
            let (bx, by) = ((l.body.x + (l.card.x - l.body.x) / 2.0) as u32, (l.card.y + 100.0) as u32);
            let p = out.pixel(bx, by);
            assert!(p[0] < 60 && p[1] < 60 && p[2] < 60, "{device:?}: bezel at {bx},{by} is {p:?}");
            assert_eq!(out.pixel((l.pic.x + 150.0) as u32, (l.pic.y + 100.0) as u32), BLUE, "{device:?}: the picture");
            assert_eq!(out.pixel(2, 2), [255, 255, 255, 255], "{device:?}: the background");
            if matches!(device, Device::Laptop | Device::Monitor) {
                // The base or the stand, light grey, below the body.
                let below = out.pixel((l.body.x + l.body.w / 2.0) as u32 - 1, (l.body.y + l.body.h + 3.0) as u32);
                assert!(below[0] > 120 && below[0] < 230 && below[2] < 240, "{device:?}: below the body {below:?}");
            }
        }
    }

    #[test]
    fn a_border_is_drawn_on_the_edge_of_the_picture() {
        let f = Frame { background: Background::Solid(Color::WHITE), padding: 10.0, corner: 0.0, shadow: 0.0, border: 4.0, border_color: Color::rgb(255, 0, 0), ..Frame::default() };
        let out = apply_frame(&Bitmap::filled(30, 30, BLUE), &f).unwrap();
        assert_eq!(out.pixel(9, 25), [255, 0, 0, 255]);
        assert_eq!(out.pixel(25, 25), BLUE);
    }
}
