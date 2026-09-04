//! Platform video presentation shared by the stream worker and ScarletUI.

use std::rc::Rc;

#[cfg(not(target_os = "scarlet"))]
use scarlet_ui::{Color, Text, ViewExt};
use scarlet_ui::{ComponentElement, Element, Event, View};

type VideoEventHandler = Rc<dyn Fn(&Event) -> bool>;

#[cfg(target_os = "scarlet")]
mod platform {
    use super::VideoEventHandler;
    use std::collections::BTreeMap;
    use std::mem;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};

    use core::simd::Simd;
    use core::simd::cmp::SimdOrd;
    use core::simd::num::{SimdInt, SimdUint};
    use scarlet_ui::{CanvasView, InvalidationKind, Listenable, SubscriptionId, View, ViewExt};

    #[derive(Clone)]
    pub(super) struct PlatformVideoOutput {
        frames: Arc<Mutex<FrameData>>,
        paint_signal: Arc<PaintSignal>,
    }

    struct FrameData {
        pixels: Vec<u8>,
        spare_pixels: Vec<u8>,
        width: u32,
        height: u32,
        timestamp_us: u64,
    }

    impl FrameData {
        fn new() -> Self {
            Self {
                pixels: Vec::new(),
                spare_pixels: Vec::new(),
                width: 0,
                height: 0,
                timestamp_us: 0,
            }
        }
    }

    struct PaintSignal {
        next_subscription: AtomicU32,
        subscribers: Mutex<BTreeMap<SubscriptionId, Arc<dyn Fn() + Send + Sync>>>,
    }

    impl PaintSignal {
        fn new() -> Self {
            Self {
                next_subscription: AtomicU32::new(0),
                subscribers: Mutex::new(BTreeMap::new()),
            }
        }

        fn notify(&self) {
            let subscribers = lock(&self.subscribers);
            for callback in subscribers.values() {
                callback();
            }
        }
    }

    impl Listenable for PaintSignal {
        fn subscribe_any(&self, callback: Arc<dyn Fn() + Send + Sync>) -> SubscriptionId {
            let id = SubscriptionId::new(self.next_subscription.fetch_add(1, Ordering::Relaxed));
            lock(&self.subscribers).insert(id, callback);
            id
        }

        fn unsubscribe(&self, id: SubscriptionId) -> bool {
            lock(&self.subscribers).remove(&id).is_some()
        }

        fn invalidation_kind(&self) -> InvalidationKind {
            InvalidationKind::Paint
        }
    }

    impl PlatformVideoOutput {
        pub(super) fn new() -> Self {
            Self {
                frames: Arc::new(Mutex::new(FrameData::new())),
                paint_signal: Arc::new(PaintSignal::new()),
            }
        }

        pub(super) fn reset(&self) {
            let mut frame = lock(&self.frames);
            frame.pixels.clear();
            frame.width = 0;
            frame.height = 0;
            frame.timestamp_us = 0;
            drop(frame);
            self.paint_signal.notify();
        }

        pub(super) fn present_nv12(
            &self,
            width: u32,
            height: u32,
            timestamp_us: u64,
            nv12: &[u8],
        ) -> Result<(), String> {
            if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
                return Err(format!("unsupported NV12 dimensions: {width}x{height}"));
            }
            let width_usize = width as usize;
            let height_usize = height as usize;
            let y_len = width_usize
                .checked_mul(height_usize)
                .ok_or_else(|| String::from("decoded frame dimensions overflow"))?;
            let uv_len = width_usize
                .checked_mul(height_usize.div_ceil(2))
                .ok_or_else(|| String::from("decoded chroma dimensions overflow"))?;
            let nv12_len = y_len
                .checked_add(uv_len)
                .ok_or_else(|| String::from("decoded frame size overflows"))?;
            if nv12.len() < nv12_len {
                return Err(format!(
                    "invalid decoded NV12 frame: {width}x{height}, {} bytes",
                    nv12.len()
                ));
            }

            let pixel_len = y_len
                .checked_mul(4)
                .ok_or_else(|| String::from("BGRA frame size overflows"))?;
            let mut pixels = {
                let mut frame = lock(&self.frames);
                mem::take(&mut frame.spare_pixels)
            };
            pixels.resize(pixel_len, 0);
            nv12_to_bgra(width, height, &nv12[..nv12_len], &mut pixels);

            let mut frame = lock(&self.frames);
            let previous = mem::replace(&mut frame.pixels, pixels);
            frame.spare_pixels = previous;
            frame.width = width;
            frame.height = height;
            frame.timestamp_us = timestamp_us;
            drop(frame);
            self.paint_signal.notify();
            Ok(())
        }

        pub(super) fn listenable(&self) -> &dyn Listenable {
            self.paint_signal.as_ref()
        }

        pub(super) fn canvas(
            &self,
            event_handler: Option<VideoEventHandler>,
        ) -> impl View + Clone + use<> {
            let frames = self.frames.clone();
            CanvasView::new(
                1280.0,
                720.0,
                Rc::new(move |buffer, width, height| {
                    draw_video_frame(buffer, width, height, &frames);
                }),
            )
            .on_event(move |event| event_handler.as_ref().is_some_and(|handler| handler(event)))
            .frame(f32::INFINITY, f32::INFINITY)
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn draw_video_frame(
        destination: &mut [u8],
        destination_width: u32,
        destination_height: u32,
        frames: &Mutex<FrameData>,
    ) {
        fill_bgra(destination, [0, 0, 0, 255]);
        if destination_width == 0 || destination_height == 0 {
            return;
        }

        let frame = lock(frames);
        if frame.width == 0 || frame.height == 0 || frame.pixels.is_empty() {
            return;
        }

        let (draw_width, draw_height) = fit_size(
            frame.width,
            frame.height,
            destination_width,
            destination_height,
        );
        let x_offset = (destination_width - draw_width) / 2;
        let y_offset = (destination_height - draw_height) / 2;
        let destination_stride = destination_width as usize * 4;
        let source_stride = frame.width as usize * 4;

        if draw_width == frame.width && draw_height == frame.height {
            for row in 0..draw_height as usize {
                let source_start = row * source_stride;
                let destination_start =
                    (row + y_offset as usize) * destination_stride + x_offset as usize * 4;
                let row_len = draw_width as usize * 4;
                destination[destination_start..destination_start + row_len]
                    .copy_from_slice(&frame.pixels[source_start..source_start + row_len]);
            }
            return;
        }

        for destination_y in 0..draw_height {
            let source_y = (u64::from(destination_y) * u64::from(frame.height)
                / u64::from(draw_height)) as usize;
            let destination_row =
                (destination_y + y_offset) as usize * destination_stride + x_offset as usize * 4;
            let source_row = source_y * source_stride;
            for destination_x in 0..draw_width {
                let source_x = (u64::from(destination_x) * u64::from(frame.width)
                    / u64::from(draw_width)) as usize;
                let source_offset = source_row + source_x * 4;
                let destination_offset = destination_row + destination_x as usize * 4;
                destination[destination_offset..destination_offset + 4]
                    .copy_from_slice(&frame.pixels[source_offset..source_offset + 4]);
            }
        }
    }

    fn fit_size(
        source_width: u32,
        source_height: u32,
        destination_width: u32,
        destination_height: u32,
    ) -> (u32, u32) {
        let width_limited = u64::from(destination_width) * u64::from(source_height)
            <= u64::from(destination_height) * u64::from(source_width);
        if width_limited {
            let height = (u64::from(destination_width) * u64::from(source_height)
                / u64::from(source_width)) as u32;
            (destination_width, height.max(1))
        } else {
            let width = (u64::from(destination_height) * u64::from(source_width)
                / u64::from(source_height)) as u32;
            (width.max(1), destination_height)
        }
    }

    fn fill_bgra(destination: &mut [u8], color: [u8; 4]) {
        for pixel in destination.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
    }

    fn nv12_to_bgra(width: u32, height: u32, nv12: &[u8], pixels: &mut [u8]) {
        const LANES: usize = 8;

        let width = width as usize;
        let height = height as usize;
        let y_plane_len = width * height;
        let uv_plane = &nv12[y_plane_len..];

        for y in 0..height {
            let y_row = y * width;
            let uv_row = (y / 2) * width;
            let mut x = 0usize;

            while x + LANES <= width {
                let y_values = Simd::<u8, LANES>::from_slice(&nv12[y_row + x..y_row + x + LANES])
                    .cast::<i32>();
                let uv_base = uv_row + (x & !1);
                let u_values = Simd::<i32, LANES>::from_array([
                    uv_plane[uv_base] as i32,
                    uv_plane[uv_base] as i32,
                    uv_plane[uv_base + 2] as i32,
                    uv_plane[uv_base + 2] as i32,
                    uv_plane[uv_base + 4] as i32,
                    uv_plane[uv_base + 4] as i32,
                    uv_plane[uv_base + 6] as i32,
                    uv_plane[uv_base + 6] as i32,
                ]);
                let v_values = Simd::<i32, LANES>::from_array([
                    uv_plane[uv_base + 1] as i32,
                    uv_plane[uv_base + 1] as i32,
                    uv_plane[uv_base + 3] as i32,
                    uv_plane[uv_base + 3] as i32,
                    uv_plane[uv_base + 5] as i32,
                    uv_plane[uv_base + 5] as i32,
                    uv_plane[uv_base + 7] as i32,
                    uv_plane[uv_base + 7] as i32,
                ]);

                let (red, green, blue) = yuv_to_rgb_simd(y_values, u_values, v_values);
                store_bgra8(pixels, (y_row + x) * 4, red, green, blue);
                x += LANES;
            }

            while x < width {
                let y_value = nv12[y_row + x] as i32;
                let uv_offset = uv_row + (x & !1);
                let u_value = uv_plane[uv_offset] as i32;
                let v_value = uv_plane[uv_offset + 1] as i32;
                let (red, green, blue) = yuv_to_rgb(y_value, u_value, v_value);
                let offset = (y_row + x) * 4;
                pixels[offset] = blue;
                pixels[offset + 1] = green;
                pixels[offset + 2] = red;
                pixels[offset + 3] = 255;
                x += 1;
            }
        }
    }

    fn store_bgra8(
        pixels: &mut [u8],
        offset: usize,
        red: Simd<u8, 8>,
        green: Simd<u8, 8>,
        blue: Simd<u8, 8>,
    ) {
        let packed = blue.cast::<u32>()
            | (green.cast::<u32>() << Simd::splat(8))
            | (red.cast::<u32>() << Simd::splat(16))
            | Simd::splat(0xff00_0000);
        for (lane, pixel) in packed.to_array().iter().enumerate() {
            // SAFETY: `offset` identifies eight complete BGRA pixels in
            // `pixels`; unaligned writes avoid imposing a u32 alignment.
            unsafe {
                (pixels.as_mut_ptr().add(offset + lane * 4) as *mut u32).write_unaligned(*pixel);
            }
        }
    }

    fn yuv_to_rgb_simd(
        y: Simd<i32, 8>,
        u: Simd<i32, 8>,
        v: Simd<i32, 8>,
    ) -> (Simd<u8, 8>, Simd<u8, 8>, Simd<u8, 8>) {
        let c = (y - Simd::splat(16)).simd_max(Simd::splat(0));
        let d = u - Simd::splat(128);
        let e = v - Simd::splat(128);
        let rounding = Simd::splat(128);
        let red = (Simd::splat(298) * c + Simd::splat(409) * e + rounding) >> Simd::splat(8);
        let green = (Simd::splat(298) * c - Simd::splat(100) * d - Simd::splat(208) * e + rounding)
            >> Simd::splat(8);
        let blue = (Simd::splat(298) * c + Simd::splat(516) * d + rounding) >> Simd::splat(8);
        (
            clamp_u8_simd(red),
            clamp_u8_simd(green),
            clamp_u8_simd(blue),
        )
    }

    fn clamp_u8_simd(value: Simd<i32, 8>) -> Simd<u8, 8> {
        value
            .simd_clamp(Simd::splat(0), Simd::splat(255))
            .cast::<u8>()
    }

    fn yuv_to_rgb(y: i32, u: i32, v: i32) -> (u8, u8, u8) {
        let c = (y - 16).max(0);
        let d = u - 128;
        let e = v - 128;
        let red = (298 * c + 409 * e + 128) >> 8;
        let green = (298 * c - 100 * d - 208 * e + 128) >> 8;
        let blue = (298 * c + 516 * d + 128) >> 8;
        (clamp_u8(red), clamp_u8(green), clamp_u8(blue))
    }

    fn clamp_u8(value: i32) -> u8 {
        value.clamp(0, 255) as u8
    }
}

/// Shared destination for decoded stream frames.
#[derive(Clone)]
pub(crate) struct VideoOutput {
    #[cfg(target_os = "scarlet")]
    platform: platform::PlatformVideoOutput,
}

impl VideoOutput {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(target_os = "scarlet")]
            platform: platform::PlatformVideoOutput::new(),
        }
    }

    pub(crate) fn reset(&self) {
        #[cfg(target_os = "scarlet")]
        self.platform.reset();
    }

    #[cfg(target_os = "scarlet")]
    pub(crate) fn present_nv12(
        &self,
        width: u32,
        height: u32,
        timestamp_us: u64,
        nv12: &[u8],
    ) -> Result<(), String> {
        self.platform
            .present_nv12(width, height, timestamp_us, nv12)
    }

    pub(crate) fn view(&self) -> VideoSurfaceView {
        VideoSurfaceView {
            output: self.clone(),
            event_handler: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct VideoSurfaceView {
    output: VideoOutput,
    event_handler: Option<VideoEventHandler>,
}

impl VideoSurfaceView {
    pub(crate) fn on_event(mut self, handler: impl Fn(&Event) -> bool + 'static) -> Self {
        self.event_handler = Some(Rc::new(handler));
        self
    }
}

impl View for VideoSurfaceView {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            build_video_surface,
        ))
    }

    fn listenables(&self) -> Vec<&dyn scarlet_ui::Listenable> {
        #[cfg(target_os = "scarlet")]
        {
            vec![self.output.platform.listenable()]
        }
        #[cfg(not(target_os = "scarlet"))]
        {
            Vec::new()
        }
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

#[cfg(target_os = "scarlet")]
fn build_video_surface(surface: &VideoSurfaceView) -> Box<dyn View> {
    Box::new(
        surface
            .output
            .platform
            .canvas(surface.event_handler.clone()),
    )
}

#[cfg(not(target_os = "scarlet"))]
fn build_video_surface(surface: &VideoSurfaceView) -> Box<dyn View> {
    let _ = (&surface.output, &surface.event_handler);
    Box::new(
        Text::new("Video preview is only available on Scarlet")
            .font_size(18.0)
            .color(Color::WHITE)
            .frame(f32::INFINITY, f32::INFINITY)
            .background(Color::BLACK),
    )
}

#[cfg(all(test, target_os = "scarlet"))]
mod tests {
    use super::platform::fit_size;

    #[test]
    fn video_fit_preserves_aspect_ratio() {
        assert_eq!(fit_size(1920, 1080, 960, 660), (960, 540));
        assert_eq!(fit_size(1920, 1080, 800, 800), (800, 450));
    }
}
