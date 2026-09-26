//! Platform video presentation shared by the stream worker and ScarletUI.

use std::rc::Rc;

#[cfg(not(target_os = "scarlet"))]
use scarlet_ui::{Color, Text, ViewExt};
use scarlet_ui::{ComponentElement, Element, Event, View};

type VideoEventHandler = Rc<dyn Fn(&Event) -> bool>;
type VideoTouchHandler = Rc<dyn Fn(scarlet_ui::event::TouchChange, (u32, u32), (u32, u32)) -> bool>;

#[cfg(target_os = "scarlet")]
mod platform {
    use super::{VideoEventHandler, VideoTouchHandler, fit_size};
    use scarlet_os::handle::Handle;
    use std::any::Any;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};

    use scarlet_ui::element::UpdateResult;
    use scarlet_ui::renderer::PaintContext;
    use scarlet_ui::{
        ChromaLocation, Color, Element, ElementRenderObject, Event, InvalidationKind,
        LayoutConstraints, Listenable, Point, Rect, RenderElement, SgfxTexture, Size,
        SubscriptionId, View, ViewExt, YcbcrConversion, YcbcrMatrix, YcbcrRange,
    };
    use scarlet_video_client::{DecodedImage, shared_image::*};

    #[derive(Clone)]
    pub(super) struct PlatformVideoOutput {
        // A single latest-frame slot bounds latency and releases superseded
        // leases. Render objects and GPU paint commands retain their own lease.
        frames: Arc<Mutex<Option<Arc<SharedFrame>>>>,
        paint_signal: Arc<PaintSignal>,
    }

    struct SharedFrame {
        handle: Handle,
        width: u32,
        height: u32,
        conversion: YcbcrConversion,
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
            let callbacks: Vec<_> = lock(&self.subscribers).values().cloned().collect();
            for callback in callbacks {
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
                frames: Arc::new(Mutex::new(None)),
                paint_signal: Arc::new(PaintSignal::new()),
            }
        }
        pub(super) fn reset(&self) {
            *lock(&self.frames) = None;
            self.paint_signal.notify();
        }
        pub(super) fn present_image(&self, frame: DecodedImage) -> Result<(), String> {
            let descriptor = frame.descriptor();
            let color = frame.color();
            let matrix = match color.matrix {
                COLOR_MATRIX_BT601 => YcbcrMatrix::Bt601,
                // The Moonlight session explicitly requests Rec. 709 limited.
                COLOR_UNSPECIFIED | COLOR_MATRIX_BT709 => YcbcrMatrix::Bt709,
                _ => return Err(String::from("unsupported shared video color matrix")),
            };
            if !matches!(color.primaries, 0 | 1 | 5 | 6)
                || !matches!(color.transfer, 0 | 1 | 6 | 13)
                || color.range > COLOR_RANGE_FULL
                || color.chroma_x > CHROMA_MIDPOINT
                || color.chroma_y > CHROMA_MIDPOINT
            {
                return Err(String::from(
                    "unsupported shared video color space/chroma location",
                ));
            }
            let (width, height) = (descriptor.visible.width, descriptor.visible.height);
            if width == 0 || height == 0 {
                return Err(String::from("empty shared video image"));
            }
            let conversion = YcbcrConversion {
                matrix,
                range: if color.range == COLOR_RANGE_FULL {
                    YcbcrRange::Full
                } else {
                    YcbcrRange::Limited
                },
                chroma_x: if color.chroma_x == CHROMA_MIDPOINT {
                    ChromaLocation::Midpoint
                } else {
                    ChromaLocation::Cosited
                },
                chroma_y: if color.chroma_y == CHROMA_COSITED {
                    ChromaLocation::Cosited
                } else {
                    ChromaLocation::Midpoint
                },
            };
            *lock(&self.frames) = Some(Arc::new(SharedFrame {
                handle: frame.into_handle(),
                width,
                height,
                conversion,
            }));
            self.paint_signal.notify();
            Ok(())
        }
        pub(super) fn listenable(&self) -> &dyn Listenable {
            self.paint_signal.as_ref()
        }
        pub(super) fn view(
            &self,
            event_handler: Option<VideoEventHandler>,
            touch_handler: Option<VideoTouchHandler>,
        ) -> impl View + Clone + use<> {
            SharedVideoView {
                output: self.clone(),
                event_handler,
                touch_handler,
            }
            .frame(f32::INFINITY, f32::INFINITY)
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[derive(Clone)]
    struct SharedVideoView {
        output: PlatformVideoOutput,
        event_handler: Option<VideoEventHandler>,
        touch_handler: Option<VideoTouchHandler>,
    }
    impl View for SharedVideoView {
        fn create_element(&self) -> Box<dyn Element> {
            Box::new(RenderElement::new(
                self.clone(),
                SharedVideoRender {
                    view: self.clone(),
                    size: Size::new(1280.0, 720.0),
                    source: None,
                    image: None,
                },
            ))
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    struct SharedVideoRender {
        view: SharedVideoView,
        size: Size,
        source: Option<Arc<SharedFrame>>,
        // SGFX textures stay on the UI thread; only native handles cross threads.
        image: Option<Arc<SgfxTexture>>,
    }
    impl ElementRenderObject for SharedVideoRender {
        fn layout(&mut self, constraints: LayoutConstraints) -> Size {
            let width = if constraints.max_width.is_finite() {
                constraints.max_width
            } else {
                1280.0
            };
            let height = if constraints.max_height.is_finite() {
                constraints.max_height
            } else {
                720.0
            };
            self.size = Size::new(
                width.max(constraints.min_width),
                height.max(constraints.min_height),
            );
            self.size
        }
        fn size(&self) -> Size {
            self.size
        }
        fn handle_event(&mut self, event: &Event, phase: scarlet_ui::event::Phase) -> bool {
            if phase != scarlet_ui::event::Phase::Target {
                return false;
            }
            if let Event::Touch(change) = event {
                if let (Some(handler), Some(image)) = (&self.view.touch_handler, &self.image) {
                    return handler(
                        *change,
                        (self.size.width as u32, self.size.height as u32),
                        (image.width(), image.height()),
                    );
                }
            }
            self.view
                .event_handler
                .as_ref()
                .is_some_and(|handler| handler(event))
        }
        fn render(&mut self) {
            let source = lock(&self.view.output.frames).clone();
            let unchanged = match (&self.source, &source) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if unchanged {
                return;
            }
            self.image = source.as_ref().and_then(|frame| {
                let result = frame
                    .handle
                    .duplicate()
                    .map_err(|error| format!("image lease: {error:?}"))
                    .and_then(|handle| {
                        scarlet_ui::shared_nv12_texture(
                            handle,
                            frame.width,
                            frame.height,
                            frame.conversion,
                        )
                        .map_err(|error| error.to_string())
                    });
                match result {
                    Ok(texture) => Some(texture),
                    Err(error) => {
                        eprintln!("moonlight: NV12 image adoption failed: {error}");
                        None
                    }
                }
            });
            self.source = source;
        }
        fn requires_buffer_render_for_paint(&self) -> bool {
            true
        }
        fn emits_paint_extension(&self) -> bool {
            true
        }
        fn paint<'a>(&'a self, ctx: &mut PaintContext<'a>, origin: Point) -> bool {
            ctx.fill_rect(Rect::new(origin, self.size), Color::BLACK);
            if let Some(image) = &self.image {
                let (width, height) = fit_size(
                    image.width(),
                    image.height(),
                    self.size.width as u32,
                    self.size.height as u32,
                );
                if width != 0 && height != 0 {
                    let at = Point::new(
                        origin.x + (self.size.width - width as f32) * 0.5,
                        origin.y + (self.size.height - height as f32) * 0.5,
                    );
                    image.paint(ctx, Rect::new(at, Size::new(width as f32, height as f32)));
                }
            }
            true
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
        fn update(&mut self, new_view: &dyn View) -> UpdateResult {
            let Some(view) = new_view.as_any().downcast_ref::<SharedVideoView>() else {
                return UpdateResult::Replaced;
            };
            self.view = view.clone();
            UpdateResult::Updated
        }
    }
}

pub(crate) fn fit_size(
    source_width: u32,
    source_height: u32,
    destination_width: u32,
    destination_height: u32,
) -> (u32, u32) {
    if source_width == 0 || source_height == 0 || destination_width == 0 || destination_height == 0
    {
        return (0, 0);
    }
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
    pub(crate) fn present_image(
        &self,
        image: scarlet_video_client::DecodedImage,
    ) -> Result<(), String> {
        self.platform.present_image(image)
    }

    pub(crate) fn view(&self) -> VideoSurfaceView {
        VideoSurfaceView {
            output: self.clone(),
            event_handler: None,
            touch_handler: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct VideoSurfaceView {
    output: VideoOutput,
    event_handler: Option<VideoEventHandler>,
    touch_handler: Option<VideoTouchHandler>,
}

impl VideoSurfaceView {
    pub(crate) fn on_touch(
        mut self,
        handler: impl Fn(scarlet_ui::event::TouchChange, (u32, u32), (u32, u32)) -> bool + 'static,
    ) -> Self {
        self.touch_handler = Some(Rc::new(handler));
        self
    }

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
            .view(surface.event_handler.clone(), surface.touch_handler.clone()),
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

#[cfg(test)]
mod tests {
    use super::fit_size;

    #[test]
    fn video_fit_preserves_aspect_ratio() {
        assert_eq!(fit_size(1920, 1080, 960, 660), (960, 540));
        assert_eq!(fit_size(1920, 1080, 800, 800), (800, 450));
        assert_eq!(fit_size(1080, 1920, 1280, 720), (405, 720));
        assert_eq!(fit_size(1920, 1080, 1280, 720), (1280, 720));
    }

    #[test]
    fn minimized_video_has_no_draw_extent() {
        assert_eq!(fit_size(1920, 1080, 0, 720), (0, 0));
        assert_eq!(fit_size(1920, 1080, 1280, 0), (0, 0));
        assert_eq!(fit_size(0, 0, 1280, 720), (0, 0));
    }
}
