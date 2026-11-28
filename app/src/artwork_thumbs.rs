//! Decode visible track-list covers off the UI thread. Limit queued files, decoded
//! results, resident textures and the number of uploads in one viewport pass.
use crate::app::App;
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions, ViewportId};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

const QUEUE: usize = 16;
const RESIDENT: usize = 256;
const UPLOADS_PER_PASS: usize = 4;

struct Request {
    path: String,
    generation: Arc<()>,
    viewport: ViewportId,
}

struct Worker {
    jobs: Sender<Request>,
    results: Receiver<(Request, Option<ColorImage>)>,
}

fn decode(path: &str) -> Option<ColorImage> {
    let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let small = reader.decode().ok()?.thumbnail(96, 96).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], &small))
}

impl Worker {
    fn start(ctx: Context) -> Option<Self> {
        let (jobs, requests) = bounded::<Request>(QUEUE);
        let (completed, results) = bounded(QUEUE);
        std::thread::Builder::new().name("recordbox-cover".into()).spawn(move || {
            for request in requests {
                let image = decode(&request.path);
                let viewport = request.viewport;
                if completed.send((request, image)).is_err() { break; }
                ctx.request_repaint_of(viewport);
                if viewport != ViewportId::ROOT { ctx.request_repaint_of(ViewportId::ROOT); }
            }
        }).ok()?;
        Some(Self { jobs, results })
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    worker: OnceLock<Option<Worker>>,
    pending: HashMap<String, Arc<()>>,
    textures: HashMap<String, (Option<TextureHandle>, u64)>,
    clock: u64,
    last_pass: Option<(ViewportId, u64)>,
}

impl Cache {
    pub(crate) fn forget(&mut self, path: &str) {
        self.pending.remove(path);
        self.textures.remove(path);
    }

    fn finish(&mut self, ctx: &Context) {
        let pass = (ctx.viewport_id(), ctx.cumulative_pass_nr());
        if self.last_pass == Some(pass) { return; }
        self.last_pass = Some(pass);
        let Some(Some(worker)) = self.worker.get() else { return; };
        for (request, image) in worker.results.try_iter().take(UPLOADS_PER_PASS) {
            if !self.pending.get(&request.path).is_some_and(|p| Arc::ptr_eq(p, &request.generation)) {
                continue; // The user replaced the artwork while this file was being read.
            }
            self.pending.remove(&request.path);
            let texture = image.map(|image| {
                crate::ui_feel::art_ready(&request.path);
                ctx.load_texture(&request.path, image, TextureOptions::LINEAR)
            });
            if self.textures.len() >= RESIDENT {
                let oldest = self.textures.iter().min_by_key(|(_, (_, touched))| *touched).map(|(p, _)| p.clone());
                if let Some(path) = oldest { self.textures.remove(&path); }
            }
            self.textures.insert(request.path, (texture, self.clock));
        }
        if !worker.results.is_empty() { ctx.request_repaint(); }
    }

    fn lookup(&mut self, ctx: &Context, path: &str) -> Option<TextureHandle> {
        self.finish(ctx);
        self.clock = self.clock.wrapping_add(1);
        if let Some((texture, touched)) = self.textures.get_mut(path) {
            *touched = self.clock;
            return texture.clone();
        }
        if self.pending.contains_key(path) { return None; }
        let worker = self.worker.get_or_init(|| Worker::start(ctx.clone())).as_ref()?;
        let generation = Arc::new(());
        let request = Request { path: path.into(), generation: generation.clone(), viewport: ctx.viewport_id() };
        if worker.jobs.try_send(request).is_ok() {
            self.pending.insert(path.into(), generation);
        }
        None
    }
}

pub(crate) fn get(app: &mut App, path: &str) -> Option<TextureHandle> {
    if path.is_empty() { return None; }
    if let Some(texture) = app.textures.get(path) { return texture.clone(); }
    app.list_artwork.lookup(&app.ctx, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_never_blocks_on_a_full_queue_and_old_art_cannot_replace_an_edit() {
        let ctx = Context::default();
        let (tx, requests) = bounded(2);
        let (done, rx) = bounded(8);
        let mut cache = Cache::default();
        assert!(cache.worker.set(Some(Worker { jobs: tx, results: rx })).is_ok());
        let _ = ctx.run(Default::default(), |ctx| {
            for path in ["edited", "visible", "overflow", "visible"] {
                cache.lookup(ctx, path);
            }
        });
        assert_eq!(cache.pending.len(), 2);
        assert!(!cache.pending.contains_key("overflow"));
        let old = requests.try_recv().unwrap();
        cache.forget("edited");
        let _ = ctx.run(Default::default(), |ctx| { cache.lookup(ctx, "edited"); });
        let other = requests.try_recv().unwrap();
        let replacement = requests.try_recv().unwrap();
        assert!(done.try_send((old, None)).is_ok());
        let _ = ctx.run(Default::default(), |ctx| cache.finish(ctx));
        assert!(Arc::ptr_eq(&cache.pending["edited"], &replacement.generation));
        assert!(!cache.textures.contains_key("edited"));
        assert!(done.try_send((other, None)).is_ok());
        assert!(done.try_send((replacement, None)).is_ok());
        let _ = ctx.run(Default::default(), |ctx| cache.finish(ctx));
        assert!(cache.pending.is_empty());
        assert!(cache.textures.contains_key("edited"));
    }

    #[test]
    fn uploads_are_bounded_per_pass_and_resident_entries_are_evicted() {
        let ctx = Context::default();
        let (tx, _requests) = bounded(1);
        let (done, rx) = bounded(8);
        let mut cache = Cache::default();
        assert!(cache.worker.set(Some(Worker { jobs: tx, results: rx })).is_ok());
        for n in 0..RESIDENT {
            cache.textures.insert(n.to_string(), (None, n as u64));
        }
        cache.clock = RESIDENT as u64;
        for n in 0..UPLOADS_PER_PASS + 1 {
            let path = format!("new-{n}");
            let generation = Arc::new(());
            cache.pending.insert(path.clone(), generation.clone());
            assert!(done.try_send((Request { path, generation, viewport: ViewportId::ROOT }, None)).is_ok());
        }
        let _ = ctx.run(Default::default(), |ctx| {
            cache.finish(ctx);
            cache.finish(ctx);
        });
        assert_eq!(cache.pending.len(), 1);
        assert_eq!(cache.textures.len(), RESIDENT);
        assert!(!cache.textures.contains_key("0"));
        let _ = ctx.run(Default::default(), |ctx| cache.finish(ctx));
        assert!(cache.pending.is_empty());
        assert_eq!(cache.textures.len(), RESIDENT);
    }

    #[test]
    fn decoder_preserves_aspect_ratio_and_drops_unreadable_covers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cover.png");
        let path = path.to_str().unwrap();
        assert!(decode(path).is_none());
        image::RgbImage::new(320, 160).save(path).unwrap();
        assert_eq!(decode(path).unwrap().size, [96, 48]);
        std::fs::write(path, b"broken cover").unwrap();
        assert!(decode(path).is_none());
    }
}
