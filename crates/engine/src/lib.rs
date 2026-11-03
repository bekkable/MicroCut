//! Session state for MicroCut (layer L2): the open video, the cursor, the selected range and the
//! timeline view, plus the geometry the timeline draws (scale marks, thumbnail tiles).
//!
//! Everything here is independent of the UI toolkit and tested headless. Edit positions are
//! whole frames (`i64`); `f64` is used only for drawing (pixels, fractional frame positions).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod scale;
mod session;
mod tiles;
mod view;

pub use scale::{Mark, frame_marks, snap_boundary, time_marks};
pub use session::{FrameRange, RangeEnd, Session, Side};
pub use tiles::{Tile, tiles};
pub use view::{MAX_PX_PER_FRAME, View};
