use gtk::{glib, prelude::*, subclass::prelude::*};

use crate::engine::Crop;

mod imp {
    use super::*;
    use glib::subclass::Signal;
    use gtk::{gdk, graphene, gsk};
    use std::{cell::Cell, sync::OnceLock};

    /// Distance in pixels from a corner or edge within which it can be dragged.
    const GRAB_DISTANCE: f64 = 14.;
    /// Length and thickness of the corner handles.
    const HANDLE_LENGTH: f32 = 18.;
    const HANDLE_WIDTH: f32 = 4.;

    /// What a drag moves: each flag is an edge that follows the pointer.
    #[derive(Debug, Default, Clone, Copy, PartialEq)]
    pub(super) struct Grab {
        left: bool,
        top: bool,
        right: bool,
        bottom: bool,
    }

    impl Grab {
        fn moves_whole(&self) -> bool {
            self.left && self.top && self.right && self.bottom
        }

        fn cursor(&self) -> &'static str {
            match (self.left, self.top, self.right, self.bottom) {
                (true, true, true, true) => "move",
                (true, true, false, false) | (false, false, true, true) => "nwse-resize",
                (false, true, true, false) | (true, false, false, true) => "nesw-resize",
                (true, false, false, false) | (false, false, true, false) => "ew-resize",
                (false, true, false, false) | (false, false, false, true) => "ns-resize",
                _ => "default",
            }
        }
    }

    #[derive(Debug, Default)]
    pub struct VtCropOverlay {
        pub(super) crop: Cell<Crop>,
        /// Width over height of the whole video frame.
        pub(super) aspect: Cell<f64>,
        grab: Cell<Option<Grab>>,
        drag_start_crop: Cell<Crop>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VtCropOverlay {
        const NAME: &'static str = "VtCropOverlay";
        type Type = super::VtCropOverlay;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("vt-crop-overlay");
        }
    }

    impl ObjectImpl for VtCropOverlay {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<[Signal; 1]> = OnceLock::new();
            SIGNALS.get_or_init(|| [Signal::builder("changed").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            self.aspect.set(16. / 9.);

            let drag = gtk::GestureDrag::new();
            drag.connect_drag_begin({
                let obj = obj.downgrade();
                move |gesture, x, y| {
                    let obj = obj.upgrade().unwrap();
                    let imp = obj.imp();
                    let grab = imp.grab_at(x, y);
                    if grab.is_none() {
                        gesture.set_state(gtk::EventSequenceState::Denied);
                        return;
                    }
                    imp.grab.set(grab);
                    imp.drag_start_crop.set(imp.crop.get());
                }
            });
            drag.connect_drag_update({
                let obj = obj.downgrade();
                move |_, dx, dy| {
                    let obj = obj.upgrade().unwrap();
                    obj.imp().on_drag(dx, dy);
                }
            });
            drag.connect_drag_end({
                let obj = obj.downgrade();
                move |_, _, _| {
                    let obj = obj.upgrade().unwrap();
                    obj.imp().grab.set(None);
                }
            });
            obj.add_controller(drag);

            let click = gtk::GestureClick::new();
            click.connect_pressed({
                let obj = obj.downgrade();
                move |_, n_press, _, _| {
                    if n_press == 2 {
                        let obj = obj.upgrade().unwrap();
                        obj.imp().set_crop(Crop::default());
                        obj.emit_by_name::<()>("changed", &[]);
                    }
                }
            });
            obj.add_controller(click);

            let motion = gtk::EventControllerMotion::new();
            motion.connect_motion({
                let obj = obj.downgrade();
                move |_, x, y| {
                    let obj = obj.upgrade().unwrap();
                    let imp = obj.imp();
                    let grab = imp.grab.get().or_else(|| imp.grab_at(x, y));
                    obj.set_cursor_from_name(grab.map(|grab| grab.cursor()));
                }
            });
            obj.add_controller(motion);
        }
    }

    impl WidgetImpl for VtCropOverlay {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let crop = self.crop_rect();
            let width = self.obj().width() as f32;
            let height = self.obj().height() as f32;

            // Darken everything outside the kept part, including the bars around the video.
            let shade = gdk::RGBA::new(0., 0., 0., 0.6);
            for rect in [
                graphene::Rect::new(0., 0., width, crop.y()),
                graphene::Rect::new(0., crop.y() + crop.height(), width, height),
                graphene::Rect::new(0., crop.y(), crop.x(), crop.height()),
                graphene::Rect::new(
                    crop.x() + crop.width(),
                    crop.y(),
                    width,
                    crop.height(),
                ),
            ] {
                snapshot.append_color(&shade, &rect);
            }

            let guide = gdk::RGBA::new(1., 1., 1., 0.35);
            for i in 1..3 {
                let fraction = i as f32 / 3.;
                snapshot.append_color(
                    &guide,
                    &graphene::Rect::new(
                        crop.x() + crop.width() * fraction,
                        crop.y(),
                        1.,
                        crop.height(),
                    ),
                );
                snapshot.append_color(
                    &guide,
                    &graphene::Rect::new(
                        crop.x(),
                        crop.y() + crop.height() * fraction,
                        crop.width(),
                        1.,
                    ),
                );
            }

            let white = gdk::RGBA::WHITE;
            snapshot.append_border(
                &gsk::RoundedRect::from_rect(crop, 0.),
                &[1.5; 4],
                &[white; 4],
            );

            let (x0, y0) = (crop.x(), crop.y());
            let (x1, y1) = (crop.x() + crop.width(), crop.y() + crop.height());
            let length = HANDLE_LENGTH.min(crop.width() / 2.).min(crop.height() / 2.);
            for (x, y, sx, sy) in [
                (x0, y0, 1., 1.),
                (x1, y0, -1., 1.),
                (x0, y1, 1., -1.),
                (x1, y1, -1., -1.),
            ] {
                let outer_x = x - sx * HANDLE_WIDTH / 2.;
                let outer_y = y - sy * HANDLE_WIDTH / 2.;
                let rect = |x: f32, y: f32, w: f32, h: f32| {
                    graphene::Rect::new(x.min(x + w), y.min(y + h), w.abs(), h.abs())
                };
                snapshot.append_color(&white, &rect(outer_x, outer_y, sx * length, sy * HANDLE_WIDTH));
                snapshot.append_color(&white, &rect(outer_x, outer_y, sx * HANDLE_WIDTH, sy * length));
            }
        }
    }

    impl VtCropOverlay {
        pub(super) fn set_crop(&self, crop: Crop) {
            self.crop.set(crop.clamped());
            self.obj().queue_draw();
        }

        /// Where the video is drawn, fitted into the widget like a `gtk::Picture` does.
        fn video_rect(&self) -> graphene::Rect {
            let width = self.obj().width() as f64;
            let height = self.obj().height() as f64;
            let aspect = self.aspect.get();
            let (video_width, video_height) = if width / height > aspect {
                (height * aspect, height)
            } else {
                (width, width / aspect)
            };
            graphene::Rect::new(
                ((width - video_width) / 2.) as f32,
                ((height - video_height) / 2.) as f32,
                video_width as f32,
                video_height as f32,
            )
        }

        fn crop_rect(&self) -> graphene::Rect {
            let video = self.video_rect();
            let crop = self.crop.get();
            graphene::Rect::new(
                video.x() + video.width() * crop.x as f32,
                video.y() + video.height() * crop.y as f32,
                video.width() * crop.width as f32,
                video.height() * crop.height as f32,
            )
        }

        fn grab_at(&self, x: f64, y: f64) -> Option<Grab> {
            let rect = self.crop_rect();
            let (x0, y0) = (rect.x() as f64, rect.y() as f64);
            let (x1, y1) = (x0 + rect.width() as f64, y0 + rect.height() as f64);
            let inside_x = x > x0 - GRAB_DISTANCE && x < x1 + GRAB_DISTANCE;
            let inside_y = y > y0 - GRAB_DISTANCE && y < y1 + GRAB_DISTANCE;
            if !inside_x || !inside_y {
                return None;
            }
            let grab = Grab {
                left: (x - x0).abs() < GRAB_DISTANCE,
                top: (y - y0).abs() < GRAB_DISTANCE,
                right: (x - x1).abs() < GRAB_DISTANCE,
                bottom: (y - y1).abs() < GRAB_DISTANCE,
            };
            // On a small crop both edges are in reach; take the nearer one.
            let grab = Grab {
                left: grab.left && (!grab.right || (x - x0).abs() <= (x - x1).abs()),
                right: grab.right && (!grab.left || (x - x1).abs() < (x - x0).abs()),
                top: grab.top && (!grab.bottom || (y - y0).abs() <= (y - y1).abs()),
                bottom: grab.bottom && (!grab.top || (y - y1).abs() < (y - y0).abs()),
            };
            if grab == Grab::default() {
                Some(Grab {
                    left: true,
                    top: true,
                    right: true,
                    bottom: true,
                })
            } else {
                Some(grab)
            }
        }

        fn on_drag(&self, dx: f64, dy: f64) {
            let Some(grab) = self.grab.get() else {
                return;
            };
            let video = self.video_rect();
            let dx = dx / video.width() as f64;
            let dy = dy / video.height() as f64;
            let start = self.drag_start_crop.get();
            let crop = if grab.moves_whole() {
                Crop {
                    x: (start.x + dx).clamp(0., 1. - start.width),
                    y: (start.y + dy).clamp(0., 1. - start.height),
                    ..start
                }
            } else {
                let mut left = start.x;
                let mut top = start.y;
                let mut right = start.x + start.width;
                let mut bottom = start.y + start.height;
                if grab.left {
                    left = (left + dx).clamp(0., right - Crop::MIN_SIZE);
                }
                if grab.right {
                    right = (right + dx).clamp(left + Crop::MIN_SIZE, 1.);
                }
                if grab.top {
                    top = (top + dy).clamp(0., bottom - Crop::MIN_SIZE);
                }
                if grab.bottom {
                    bottom = (bottom + dy).clamp(top + Crop::MIN_SIZE, 1.);
                }
                Crop {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                }
            };
            self.set_crop(crop);
            self.obj().emit_by_name::<()>("changed", &[]);
        }
    }
}

glib::wrapper! {
    /// Rectangle with draggable corners and edges drawn over the video to choose a crop.
    pub struct VtCropOverlay(ObjectSubclass<imp::VtCropOverlay>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VtCropOverlay {
    pub fn crop(&self) -> Crop {
        self.imp().crop.get()
    }

    pub fn set_crop(&self, crop: Crop) {
        self.imp().set_crop(crop);
    }

    /// Sets the width over height of the whole video frame.
    pub fn set_aspect(&self, aspect: f64) {
        self.imp().aspect.set(aspect);
        self.queue_draw();
    }
}
