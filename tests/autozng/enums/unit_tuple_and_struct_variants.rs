//@ autozng

#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Nothing,
    Circle(u32),
    Rect { width: u32, height: u32 },
}
