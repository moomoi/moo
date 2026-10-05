//! Window layouts as pure geometry. Rects use a top-left origin (Accessibility's coordinates).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn overlap(&self, o: &Rect) -> f64 {
        let w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }

    fn round(self) -> Rect {
        Rect::new(self.x.round(), self.y.round(), self.w.round(), self.h.round())
    }
}

/// `(id, title)` for every layout, in menu order. `next-display`, `previous-display` and
/// `restore` need more than one screen's geometry and are handled by the caller.
pub const LAYOUTS: &[(&str, &str)] = &[
    ("left-half", "Left Half"),
    ("right-half", "Right Half"),
    ("top-half", "Top Half"),
    ("bottom-half", "Bottom Half"),
    ("top-left", "Top Left Quarter"),
    ("top-right", "Top Right Quarter"),
    ("bottom-left", "Bottom Left Quarter"),
    ("bottom-right", "Bottom Right Quarter"),
    ("left-third", "First Third"),
    ("center-third", "Center Third"),
    ("right-third", "Last Third"),
    ("left-two-thirds", "First Two Thirds"),
    ("right-two-thirds", "Last Two Thirds"),
    ("maximize", "Maximize"),
    ("almost-maximize", "Almost Maximize"),
    ("maximize-height", "Maximize Height"),
    ("center", "Center"),
    ("reasonable-size", "Reasonable Size"),
    ("next-display", "Next Display"),
    ("previous-display", "Previous Display"),
    ("restore", "Restore"),
];

pub fn title(id: &str) -> Option<&'static str> {
    LAYOUTS.iter().find(|l| l.0 == id).map(|l| l.1)
}

/// Where a window at `cur` goes for `layout` on a screen whose usable area is `v`.
pub fn frame(layout: &str, v: Rect, cur: Rect) -> Option<Rect> {
    let (hw, hh) = (v.w / 2.0, v.h / 2.0);
    let t = v.w / 3.0;
    let r = match layout {
        "left-half" => Rect::new(v.x, v.y, hw, v.h),
        "right-half" => Rect::new(v.x + hw, v.y, v.w - hw, v.h),
        "top-half" => Rect::new(v.x, v.y, v.w, hh),
        "bottom-half" => Rect::new(v.x, v.y + hh, v.w, v.h - hh),
        "top-left" => Rect::new(v.x, v.y, hw, hh),
        "top-right" => Rect::new(v.x + hw, v.y, v.w - hw, hh),
        "bottom-left" => Rect::new(v.x, v.y + hh, hw, v.h - hh),
        "bottom-right" => Rect::new(v.x + hw, v.y + hh, v.w - hw, v.h - hh),
        "left-third" => Rect::new(v.x, v.y, t, v.h),
        "center-third" => Rect::new(v.x + t, v.y, t, v.h),
        "right-third" => Rect::new(v.x + 2.0 * t, v.y, v.w - 2.0 * t, v.h),
        "left-two-thirds" => Rect::new(v.x, v.y, 2.0 * t, v.h),
        "right-two-thirds" => Rect::new(v.x + t, v.y, v.w - t, v.h),
        "maximize" => v,
        "almost-maximize" => centered(v, v.w * 0.9, v.h * 0.9),
        "maximize-height" => Rect::new(cur.x.clamp(v.x, (v.x + v.w - cur.w.min(v.w)).max(v.x)), v.y, cur.w.min(v.w), v.h),
        "center" => centered(v, cur.w.min(v.w), cur.h.min(v.h)),
        "reasonable-size" => centered(v, (v.w * 0.6).max(cur.w.min(v.w * 0.6)), v.h * 0.7),
        _ => return None,
    };
    Some(r.round())
}

fn centered(v: Rect, w: f64, h: f64) -> Rect {
    Rect::new(v.x + (v.w - w) / 2.0, v.y + (v.h - h) / 2.0, w, h)
}

/// The same relative place and size on another screen.
pub fn move_to_screen(cur: Rect, from: Rect, to: Rect) -> Rect {
    let sx = to.w / from.w;
    let sy = to.h / from.h;
    let w = (cur.w * sx).min(to.w);
    let h = (cur.h * sy).min(to.h);
    let x = (to.x + (cur.x - from.x) * sx).clamp(to.x, to.x + to.w - w);
    let y = (to.y + (cur.y - from.y) * sy).clamp(to.y, to.y + to.h - h);
    Rect::new(x, y, w, h).round()
}

/// The screen (index) that holds most of `r`, or whose center is nearest.
pub fn screen_for(r: Rect, screens: &[Rect]) -> usize {
    let best = (0..screens.len()).max_by(|&a, &b| screens[a].overlap(&r).total_cmp(&screens[b].overlap(&r)));
    match best {
        Some(i) if screens[i].overlap(&r) > 0.0 => i,
        _ => {
            let (cx, cy) = r.center();
            (0..screens.len())
                .min_by(|&a, &b| {
                    let d = |s: &Rect| {
                        let (x, y) = s.center();
                        (x - cx).powi(2) + (y - cy).powi(2)
                    };
                    d(&screens[a]).total_cmp(&d(&screens[b]))
                })
                .unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_tile_the_screen() {
        let v = Rect::new(0.0, 25.0, 1512.0, 920.0);
        let cur = Rect::new(100.0, 100.0, 800.0, 600.0);
        assert_eq!(frame("left-half", v, cur), Some(Rect::new(0.0, 25.0, 756.0, 920.0)));
        assert_eq!(frame("right-half", v, cur), Some(Rect::new(756.0, 25.0, 756.0, 920.0)));
        assert_eq!(frame("bottom-right", v, cur), Some(Rect::new(756.0, 485.0, 756.0, 460.0)));
        assert_eq!(frame("right-third", v, cur), Some(Rect::new(1008.0, 25.0, 504.0, 920.0)));
        assert_eq!(frame("center", v, cur), Some(Rect::new(356.0, 185.0, 800.0, 600.0)));
        assert_eq!(frame("maximize", v, cur), Some(v));
        assert_eq!(frame("maximize-height", v, cur), Some(Rect::new(100.0, 25.0, 800.0, 920.0)));
        assert_eq!(frame("teleport", v, cur), None);
        for (id, _) in LAYOUTS.iter().filter(|l| !matches!(l.0, "next-display" | "previous-display" | "restore")) {
            let f = frame(id, v, cur).unwrap();
            assert!(f.x >= v.x && f.y >= v.y && f.x + f.w <= v.x + v.w + 0.5 && f.y + f.h <= v.y + v.h + 0.5, "{id}: {f:?}");
        }
    }

    #[test]
    fn displays() {
        let a = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let b = Rect::new(1000.0, -200.0, 2000.0, 1200.0);
        let w = Rect::new(500.0, 400.0, 500.0, 400.0);
        assert_eq!(screen_for(w, &[a, b]), 0);
        assert_eq!(screen_for(Rect::new(1500.0, 0.0, 100.0, 100.0), &[a, b]), 1);
        assert_eq!(screen_for(Rect::new(-5000.0, 0.0, 10.0, 10.0), &[a, b]), 0, "off-screen goes to the nearest");
        assert_eq!(move_to_screen(w, a, b), Rect::new(2000.0, 400.0, 1000.0, 600.0));
    }
}
