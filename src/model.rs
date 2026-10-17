use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            w: self.right().max(other.right()) - x,
            h: self.bottom().max(other.bottom()) - y,
        }
    }
}
fn default_text() -> String {
    "Click here".into()
}
fn default_style() -> String {
    "arrow".into()
}
fn default_color() -> String {
    "auto".into()
}
fn default_duration() -> u64 {
    0
}
fn yes() -> bool {
    true
}
fn one() -> u32 {
    1
}
fn default_arrow_scale() -> f32 {
    1.6
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Hint {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_text")]
    pub text: String,
    #[serde(default)]
    pub panel_x: Option<i32>,
    #[serde(default)]
    pub panel_y: Option<i32>,
    #[serde(default)]
    pub panel_width: Option<u32>,
    #[serde(default)]
    pub text_page: u32,
    #[serde(default = "default_style")]
    pub style: String,
    #[serde(default)]
    pub shape: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default = "default_duration")]
    pub duration_ms: u64,
    #[serde(default = "one")]
    pub step: u32,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub background_colors: Vec<String>,
    #[serde(default)]
    pub arrow: Option<bool>,
    #[serde(default = "default_arrow_scale")]
    pub arrow_scale: f32,
    #[serde(default = "yes")]
    pub target: bool,
    #[serde(default = "yes")]
    pub animate: bool,
    /// Follow the native app window under the target without another agent call.
    #[serde(default = "yes")]
    pub follow_window: bool,
    #[serde(default)]
    pub points: Vec<Point>,
}
impl Hint {
    pub fn parse(mut value: Value) -> Result<Self, String> {
        if let Some(obj) = value.as_object_mut() {
            obj.remove("command");
        }
        let hint: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if !["arrow", "box"].contains(&hint.style.as_str()) {
            return Err("style must be arrow or box".into());
        }
        if !["ellipse", "rectangle", "rounded", "polygon"].contains(&hint.shape()) {
            return Err("Unsupported shape".into());
        }
        if hint.title.chars().count() > 100 || hint.title.contains('\0') {
            return Err("title must be at most 100 characters without NUL".into());
        }
        if hint.text.chars().count() > 16000 || hint.text.contains('\0') {
            return Err("text must be at most 16000 characters without NUL".into());
        }
        if hint.panel_x.is_some() != hint.panel_y.is_some() {
            return Err("panel_x and panel_y must be supplied together".into());
        }
        if hint
            .panel_width
            .is_some_and(|width| !(120..=4000).contains(&width))
        {
            return Err("panel_width must be 120..4000 physical pixels".into());
        }
        if hint.duration_ms != 0 && !(500..=300000).contains(&hint.duration_ms) {
            return Err("duration_ms must be 0 or 500..300000".into());
        }
        if !(1..=99).contains(&hint.step) {
            return Err("step must be 1..99".into());
        }
        if !hint.arrow_scale.is_finite() || !(0.75..=2.5).contains(&hint.arrow_scale) {
            return Err("arrow_scale must be a number between 0.75 and 2.5".into());
        }
        for size in [hint.width, hint.height].into_iter().flatten() {
            if !(1..=4000).contains(&size) {
                return Err("width/height must be 1..4000".into());
            }
        }
        if hint.style == "box" && (hint.width.is_none() || hint.height.is_none()) {
            return Err("box requires width and height".into());
        }
        if hint.color != "auto" {
            parse_color(&hint.color)?;
        }
        if hint.background_colors.len() > 16 {
            return Err("background_colors accepts at most 16 colors".into());
        }
        for c in &hint.background_colors {
            parse_color(c)?;
        }
        if hint.shape() == "polygon" {
            if !(3..=64).contains(&hint.points.len()) {
                return Err("polygon requires 3..64 points".into());
            }
            if !point_in_polygon(hint.x as f32, hint.y as f32, &hint.points) {
                return Err("x,y must be inside the polygon".into());
            }
        } else if !hint.points.is_empty() {
            return Err("points requires shape polygon".into());
        }
        let b = hint.bounds();
        if b.w <= 0. || b.h <= 0. || b.w > 4000. || b.h > 4000. {
            return Err("Target bounds must be nonzero and at most 4000 pixels".into());
        }
        Ok(hint)
    }
    pub fn shape(&self) -> &str {
        self.shape.as_deref().unwrap_or(if self.style == "box" {
            "rectangle"
        } else {
            "ellipse"
        })
    }
    pub fn has_arrow(&self) -> bool {
        self.arrow.unwrap_or(self.style != "box")
    }
    pub fn bounds(&self) -> Rect {
        if self.shape() == "polygon" {
            let left = self.points.iter().map(|p| p.x).min().unwrap_or(self.x) as f32;
            let top = self.points.iter().map(|p| p.y).min().unwrap_or(self.y) as f32;
            let right = self.points.iter().map(|p| p.x).max().unwrap_or(self.x) as f32;
            let bottom = self.points.iter().map(|p| p.y).max().unwrap_or(self.y) as f32;
            Rect {
                x: left,
                y: top,
                w: right - left,
                h: bottom - top,
            }
        } else {
            let w = self.width.unwrap_or(16) as f32;
            let h = self.height.unwrap_or(16) as f32;
            Rect {
                x: self.x as f32 - if self.style == "box" { 0. } else { w / 2. },
                y: self.y as f32 - if self.style == "box" { 0. } else { h / 2. },
                w,
                h,
            }
        }
    }
    pub fn anchor(&self) -> (f32, f32) {
        if self.style == "box" && self.shape() != "polygon" {
            let b = self.bounds();
            (b.x + b.w / 2., b.y + b.h / 2.)
        } else {
            (self.x as f32, self.y as f32)
        }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let b = self.bounds();
        match self.shape() {
            "polygon" => point_in_polygon(x, y, &self.points),
            "ellipse" => {
                ((x - b.x - b.w / 2.) / (b.w / 2.)).powi(2)
                    + ((y - b.y - b.h / 2.) / (b.h / 2.)).powi(2)
                    <= 1.
            }
            "rounded" => {
                let r = 6f32.min(b.w / 2.).min(b.h / 2.);
                let cx = x.clamp(b.x + r, b.right() - r);
                let cy = y.clamp(b.y + r, b.bottom() - r);
                (x - cx).powi(2) + (y - cy).powi(2) <= r * r
            }
            _ => b.contains(x, y),
        }
    }
}
pub fn parse_color(s: &str) -> Result<[u8; 3], String> {
    if s.len() != 7 || !s.starts_with('#') || !s[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("color must be auto or #RRGGBB".into());
    }
    Ok([
        u8::from_str_radix(&s[1..3], 16).unwrap(),
        u8::from_str_radix(&s[3..5], 16).unwrap(),
        u8::from_str_radix(&s[5..7], 16).unwrap(),
    ])
}
fn point_in_polygon(x: f32, y: f32, ps: &[Point]) -> bool {
    if ps.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = ps.len() - 1;
    for i in 0..ps.len() {
        let a = ps[i];
        let b = ps[j];
        if ((a.y as f32 > y) != (b.y as f32 > y))
            && x < (b.x as f32 - a.x as f32) * (y - a.y as f32) / (b.y as f32 - a.y as f32)
                + a.x as f32
        {
            inside = !inside
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn optional_title_preserves_legacy_text_and_validates_character_count() {
        let legacy = Hint::parse(json!({"x":100,"y":100,"text":"Open settings"})).unwrap();
        assert!(legacy.title.is_empty());
        assert_eq!(legacy.text, "Open settings");
        let title_only =
            Hint::parse(json!({"x":100,"y":100,"title":"Open settings","text":""})).unwrap();
        assert_eq!(title_only.title, "Open settings");
        assert!(title_only.text.is_empty());
        assert!(Hint::parse(json!({"x":100,"y":100,"title":"é".repeat(100)})).is_ok());
        for title in [
            json!("a".repeat(101)),
            json!("Bad\0title"),
            json!(false),
            Value::Null,
        ] {
            assert!(Hint::parse(json!({"x":100,"y":100,"title":title})).is_err());
        }
    }

    #[test]
    fn independent_panel_fields_and_text_limit_are_bounded() {
        assert!(Hint::parse(json!({"x":0,"y":0,"text":"é".repeat(16000),"panel_x":-300,"panel_y":20,"panel_width":120,"text_page":u32::MAX})).is_ok());
        for fields in [
            json!({"panel_x":10}),
            json!({"panel_y":10}),
            json!({"panel_width":119}),
            json!({"panel_width":4001}),
            json!({"text_page":-1}),
            json!({"text":"a".repeat(16001)}),
        ] {
            let mut value = json!({"x":0,"y":0});
            value
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            assert!(Hint::parse(value).is_err());
        }
    }
    #[test]
    fn geometry_and_independence() {
        let h = Hint::parse(
            json!({"x":100,"y":100,"arrow":false,"shape":"rounded","width":80,"height":30}),
        )
        .unwrap();
        assert!(!h.has_arrow());
        assert!(h.target);
        assert!(h.contains(100., 100.));
        assert!(!h.contains(60., 85.));
    }
    #[test]
    fn rejects_invalid_inputs() {
        for v in [
            json!({"x":1.5,"y":2}),
            json!({"x":1,"y":2,"duration_ms":499}),
            json!({"x":1,"y":2,"color":"#xxxxxx"}),
            json!({"x":1,"y":2,"arrow":"false"}),
            json!({"x":1,"y":2,"shape":"polygon","points":[]}),
        ] {
            assert!(Hint::parse(v).is_err());
        }
    }
    #[test]
    fn polygon_anchor() {
        let h=Hint::parse(json!({"x":50,"y":50,"shape":"polygon","points":[{"x":0,"y":0},{"x":100,"y":0},{"x":100,"y":100},{"x":0,"y":100}]})).unwrap();
        assert!(h.contains(10., 10.));
        assert!(!h.contains(150., 10.));
    }
    #[test]
    fn arrow_scale_defaults_and_limits() {
        let default = Hint::parse(json!({"x":100,"y":100})).unwrap();
        assert_eq!(default.arrow_scale, 1.6);
        for scale in [0.75, 1.0, 1.3, 1.6, 2.5] {
            assert!(Hint::parse(json!({"x":100,"y":100,"arrow_scale":scale})).is_ok());
        }
        for scale in [
            json!(0.74),
            json!(2.51),
            json!(-1),
            json!("1.3"),
            Value::Null,
        ] {
            assert!(Hint::parse(json!({"x":100,"y":100,"arrow_scale":scale})).is_err());
        }
    }
    #[test]
    fn hints_persist_by_default_and_expiry_is_explicit() {
        let hint = Hint::parse(json!({"x":100,"y":100})).unwrap();
        assert_eq!(hint.duration_ms, 0);
        for duration in [0, 500, 15000, 300000] {
            let hint = Hint::parse(json!({"x":100,"y":100,"duration_ms":duration})).unwrap();
            assert_eq!(hint.duration_ms, duration);
        }
        for duration in [json!(1), json!(499), json!(300001), json!(-1), json!(1.5)] {
            assert!(Hint::parse(json!({"x":100,"y":100,"duration_ms":duration})).is_err());
        }
    }
}
