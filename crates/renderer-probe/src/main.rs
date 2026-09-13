//! Small release benchmark of the real Blitz/Vello path, without the app shell.
use anyrender::ImageRenderer;
use anyrender_vello_cpu::{ImageCacheConfig, VelloCpuImageRenderer};
use blitz_dom::{DocumentConfig, Point};
use blitz_html::HtmlDocument;
use blitz_paint::{PaintCache, paint_scene, paint_scene_cached};
use blitz_traits::shell::{ColorScheme, Viewport};
use std::time::{Duration, Instant};

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0)
}

fn main() {
    let text = "Ordinary <b>formatted email text</b> and more words. ";
    for (name, html) in [
        (
            "long-inline",
            format!("<body><p>{}</p></body>", text.repeat(800)),
        ),
        (
            "paragraphs",
            format!("<body>{}</body>", format!("<p>{text}</p>").repeat(250)),
        ),
        (
            "newsletter",
            include_str!("../../../resources/test-emails/revolut-precision.html").to_owned(),
        ),
    ] {
        for scale in [1.0, 2.0] {
            for cached in [false, true] {
                let width = (700.0 * scale) as u32;
                let tile_height = (512.0 * scale) as u32;
                let mut doc = HtmlDocument::from_html(
                    &html,
                    DocumentConfig {
                        viewport: Some(Viewport::new(
                            width,
                            (700.0 * scale) as u32,
                            scale,
                            ColorScheme::Light,
                        )),
                        ..Default::default()
                    },
                );
                let start = Instant::now();
                doc.resolve(0.0);
                let layout = start.elapsed();
                let cache = PaintCache::default();
                let mut painter = VelloCpuImageRenderer::with_image_cache_config(
                    width,
                    tile_height,
                    ImageCacheConfig {
                        max_bytes: 8 * 1024 * 1024,
                        max_age: 8,
                        prune_interval: 1,
                    },
                );
                let mut pixels = vec![0; (width * tile_height * 4) as usize];
                let height = doc.root_element().final_layout().size.height;
                let count = (height / 512.0).ceil().clamp(1.0, 64.0) as usize;
                let mut samples = Vec::new();
                let mut encode = Duration::ZERO;
                let mut max_rss = rss_kib();
                for pass in 0..3 {
                    for index in 0..count {
                        doc.set_viewport_scroll(Point {
                            x: 0.0,
                            y: index as f64 * 512.0,
                        });
                        painter.reset();
                        let start = Instant::now();
                        let mut scene_time = Duration::ZERO;
                        painter.render(
                            |scene| {
                                if cached {
                                    paint_scene_cached(
                                        scene,
                                        &mut doc,
                                        scale as f64,
                                        width,
                                        tile_height,
                                        0,
                                        0,
                                        &cache,
                                    );
                                } else {
                                    paint_scene(
                                        scene,
                                        &mut doc,
                                        scale as f64,
                                        width,
                                        tile_height,
                                        0,
                                        0,
                                    );
                                }
                                scene_time = start.elapsed();
                            },
                            &mut pixels,
                        );
                        if pass > 0 {
                            samples.push(start.elapsed().as_secs_f64() * 1000.0);
                            encode += scene_time;
                        }
                        max_rss = max_rss.max(rss_kib());
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!(
                    "{name} scale={scale} line_cache={cached}: layout={:.2}ms tile median={:.2}ms p95={:.2}ms max={:.2}ms mean_encode={:.2}ms sampled_peak_rss={max_rss}KiB",
                    layout.as_secs_f64() * 1000.0,
                    samples[samples.len() / 2],
                    samples[samples.len() * 95 / 100],
                    samples.last().unwrap(),
                    encode.as_secs_f64() * 1000.0 / samples.len() as f64
                );
            }
        }
    }
}
