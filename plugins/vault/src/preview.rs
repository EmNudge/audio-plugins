//! Renders the editor to an image with a small software rasteriser, so the layout can be checked
//! without a DAW: `cargo test -p vault --release preview -- --ignored`, then open
//! `/tmp/vault_preview_*.png`.

use nih_plug::prelude::*;
use nih_plug::wrapper::state::PluginState;
use nih_plug_egui::egui::{self, epaint::Primitive, Color32, TextureId};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::dsp::{Reverb, Settings, Space};
use crate::{ui, VaultParams};

/// A host that ignores everything.
struct Direct;

impl GuiContext for Direct {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Vst3
    }
    fn request_resize(&self) -> bool {
        false
    }
    unsafe fn raw_begin_set_parameter(&self, _: ParamPtr) {}
    unsafe fn raw_set_parameter_normalized(&self, _: ParamPtr, _: f32) {}
    unsafe fn raw_end_set_parameter(&self, _: ParamPtr) {}
    fn get_state(&self) -> PluginState {
        PluginState { version: String::new(), params: BTreeMap::new(), fields: BTreeMap::new() }
    }
    fn set_state(&self, _: PluginState) {}
}

struct Texture {
    size: [usize; 2],
    pixels: Vec<Color32>,
}

fn rasterise(ctx: &egui::Context, output: egui::FullOutput, textures: &mut HashMap<TextureId, Texture>, w: usize, h: usize, ppp: f32) -> Vec<[f32; 4]> {
    for (id, delta) in &output.textures_delta.set {
        let [iw, ih] = delta.image.size();
        let pixels: Vec<Color32> = match &delta.image {
            egui::ImageData::Color(c) => c.pixels.clone(),
            egui::ImageData::Font(f) => f.srgba_pixels(None).collect(),
        };
        match delta.pos {
            None => {
                textures.insert(*id, Texture { size: [iw, ih], pixels });
            }
            Some([x0, y0]) => {
                let t = textures.get_mut(id).unwrap();
                for y in 0..ih {
                    for x in 0..iw {
                        t.pixels[(y0 + y) * t.size[0] + x0 + x] = pixels[y * iw + x];
                    }
                }
            }
        }
    }
    let mut frame = vec![[0.0_f32; 4]; w * h];
    for prim in ctx.tessellate(output.shapes, ppp) {
        let Primitive::Mesh(mesh) = prim.primitive else { continue };
        let tex = &textures[&mesh.texture_id];
        let clip = prim.clip_rect;
        let (cx0, cy0, cx1, cy1) = (
            (clip.min.x * ppp).max(0.0) as i32,
            (clip.min.y * ppp).max(0.0) as i32,
            ((clip.max.x * ppp).ceil() as i32).min(w as i32),
            ((clip.max.y * ppp).ceil() as i32).min(h as i32),
        );
        for tri in mesh.indices.chunks_exact(3) {
            let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
            let p = v.map(|v| (v.pos.x * ppp, v.pos.y * ppp));
            let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
            if area.abs() < 1e-6 {
                continue;
            }
            let min_x = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor() as i32).max(cx0);
            let max_x = (p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil() as i32).min(cx1);
            let min_y = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor() as i32).max(cy0);
            let max_y = (p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil() as i32).min(cy1);
            for y in min_y..max_y {
                for x in min_x..max_x {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p[1].0 - px) * (p[2].1 - py) - (p[2].0 - px) * (p[1].1 - py)) / area;
                    let w1 = ((p[2].0 - px) * (p[0].1 - py) - (p[0].0 - px) * (p[2].1 - py)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -0.001 || w1 < -0.001 || w2 < -0.001 {
                        continue;
                    }
                    let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                    let t = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                    let tx = ((u * tex.size[0] as f32) as usize).min(tex.size[0] - 1);
                    let ty = ((t * tex.size[1] as f32) as usize).min(tex.size[1] - 1);
                    let texel = tex.pixels[ty * tex.size[0] + tx].to_array().map(|c| c as f32 / 255.0);
                    let mut col = [0.0_f32; 4];
                    for c in 0..4 {
                        let vc = w0 * v[0].color.to_array()[c] as f32 + w1 * v[1].color.to_array()[c] as f32 + w2 * v[2].color.to_array()[c] as f32;
                        col[c] = vc / 255.0 * texel[c];
                    }
                    let dst = &mut frame[y as usize * w + x as usize];
                    for c in 0..4 {
                        dst[c] = col[c] + dst[c] * (1.0 - col[3]);
                    }
                }
            }
        }
    }
    frame
}

fn write_png(path: &str, frame: &[[f32; 4]], w: usize, h: usize) {
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in frame {
        ppm.extend(px[..3].iter().map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
    }
    std::fs::write("/tmp/vault_preview.ppm", ppm).unwrap();
    let script = format!(
        "import zlib,struct\nd=open('/tmp/vault_preview.ppm','rb').read()\nhdr=d.split(b'\\n',3)\nraw=hdr[3]\nw,h={w},{h}\nrows=b''.join(b'\\0'+raw[y*w*3:(y+1)*w*3] for y in range(h))\n\ndef ch(t,b):\n    c=struct.pack('>I',len(b))+t+b\n    return c+struct.pack('>I',zlib.crc32(t+b)&0xffffffff)\nopen('{path}','wb').write(b'\\x89PNG\\r\\n\\x1a\\n'+ch(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+ch(b'IDAT',zlib.compress(rows))+ch(b'IEND',b''))\n"
    );
    assert!(std::process::Command::new("python3").arg("-c").arg(script).status().unwrap().success());
}

/// A drum hit every 1.2 s: a burst of noise with a fast decay.
fn drums(n: usize, sr: f32) -> Vec<f32> {
    let mut state = 7_u32;
    (0..n)
        .map(|i| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (state >> 8) as f32 / (1 << 23) as f32 - 1.0;
            let t = (i as f32 / sr) % 1.2;
            let body = (std::f32::consts::TAU * (60.0 + 140.0 * (-t * 30.0).exp()) * t).sin();
            (n * 0.5 + body) * (-t * 14.0).exp() * 0.7
        })
        .collect()
}

fn render(name: &str, settings: Settings, signal: bool) {
    let (w_pts, h_pts, ppp) = (760.0_f32, 620.0_f32, 2.0_f32);
    let params = Arc::new(VaultParams::new(&settings));
    let ctx_gui = Direct;
    let setter = ParamSetter::new(&ctx_gui);

    let sr = 48000.0;
    let mut reverb = Reverb::new();
    reverb.configure(sr, 2);
    let x = if signal { drums((sr * 7.0) as usize, sr) } else { vec![0.0; (sr * 7.0) as usize] };
    for (n, &v) in x.iter().enumerate() {
        if n % 32 == 0 {
            reverb.set(&settings);
        }
        reverb.process([v, v]);
    }
    let meter = reverb.meter.clone();

    let ctx = egui::Context::default();
    let mut state = ui::UiState::default();
    let mut textures = HashMap::new();
    let mut frame = Vec::new();
    for _ in 0..3 {
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w_pts, h_pts)));
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(ppp);
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(ui::background()).inner_margin(16.0))
                .show(ctx, |ui| ui::draw(ui, &params, &setter, &meter, &mut state));
        });
        frame = rasterise(&ctx, output, &mut textures, (w_pts * ppp) as usize, (h_pts * ppp) as usize, ppp);
    }
    write_png(&format!("/tmp/vault_preview_{name}.png"), &frame, (w_pts * ppp) as usize, (h_pts * ppp) as usize);
}

#[test]
#[ignore]
fn preview() {
    render("hall", Settings::default(), true);
    render("silent", Settings::default(), false);
    render("frozen", Settings { freeze: true, duck: 0.6, ..Settings::default() }, true);
    render(
        "abyss_shift",
        Settings { space: Space::Abyss, shift_hz: 18.0, decay_s: 9.0, size: 0.8, high_mult: 0.3, low_mult: 1.4, predelay_ms: 40.0, ..Settings::default() },
        true,
    );
}
