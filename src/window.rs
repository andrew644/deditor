#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Columns,
    Rows,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub cols: u32,
    pub rows: u32,
}

pub struct Window {
    pub tabs: Vec<usize>,
    pub cur: usize,
    pub view_rows: usize,
    pub view_cols: usize,
}

impl Window {
    pub fn new(tabs: Vec<usize>) -> Self {
        Self {
            tabs,
            cur: 0,
            view_rows: 24,
            view_cols: 80,
        }
    }

    pub fn cur_buf(&self) -> usize {
        self.tabs[self.cur]
    }

    pub fn show(&mut self, buf: usize) {
        match self.tabs.iter().position(|b| *b == buf) {
            Some(i) => self.cur = i,
            None => {
                self.tabs.push(buf);
                self.cur = self.tabs.len() - 1;
            }
        }
    }

    pub fn next_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.cur = (self.cur + 1) % self.tabs.len();
        }
    }

    pub fn prev_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.cur = (self.cur + self.tabs.len() - 1) % self.tabs.len();
        }
    }
}

/// The panes the windows occupy, in window order: one pane for the whole area,
/// or two side by side / stacked when the single split the editor allows is
/// open. An odd count gives the extra column to the left pane and the extra row
/// to the top one.
pub fn panes(split: Option<Axis>, area: Rect) -> Vec<Rect> {
    let Some(axis) = split else {
        return vec![area];
    };
    match axis {
        Axis::Columns => {
            let first = area.cols.div_ceil(2);
            vec![
                Rect {
                    x: area.x,
                    y: area.y,
                    cols: first,
                    rows: area.rows,
                },
                Rect {
                    x: area.x + first,
                    y: area.y,
                    cols: area.cols - first,
                    rows: area.rows,
                },
            ]
        }
        Axis::Rows => {
            let first = area.rows.div_ceil(2);
            vec![
                Rect {
                    x: area.x,
                    y: area.y,
                    cols: area.cols,
                    rows: first,
                },
                Rect {
                    x: area.x,
                    y: area.y + first,
                    cols: area.cols,
                    rows: area.rows - first,
                },
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(cols: u32, rows: u32) -> Rect {
        Rect {
            x: 0,
            y: 0,
            cols,
            rows,
        }
    }

    #[test]
    fn no_split_gives_one_pane_for_the_whole_area() {
        assert_eq!(panes(None, area(101, 40)), vec![area(101, 40)]);
    }

    #[test]
    fn columns_split_gives_the_extra_column_to_the_left() {
        assert_eq!(
            panes(Some(Axis::Columns), area(101, 40)),
            vec![
                Rect {
                    x: 0,
                    y: 0,
                    cols: 51,
                    rows: 40
                },
                Rect {
                    x: 51,
                    y: 0,
                    cols: 50,
                    rows: 40
                },
            ]
        );
    }

    #[test]
    fn rows_split_gives_the_extra_row_to_the_top() {
        assert_eq!(
            panes(Some(Axis::Rows), area(100, 41)),
            vec![
                Rect {
                    x: 0,
                    y: 0,
                    cols: 100,
                    rows: 21
                },
                Rect {
                    x: 0,
                    y: 21,
                    cols: 100,
                    rows: 20
                },
            ]
        );
    }

    #[test]
    fn window_tabs_show_and_cycle() {
        let mut w = Window::new(vec![3]);
        assert_eq!(w.cur_buf(), 3);
        w.show(5);
        assert_eq!(w.tabs, vec![3, 5]);
        assert_eq!(w.cur_buf(), 5);
        w.show(3);
        assert_eq!(w.tabs, vec![3, 5]);
        assert_eq!(w.cur_buf(), 3);
        w.next_tab();
        assert_eq!(w.cur_buf(), 5);
        w.prev_tab();
        w.prev_tab();
        assert_eq!(w.cur_buf(), 5);
    }
}
