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

#[derive(PartialEq, Debug)]
pub enum Node {
    Leaf(usize),
    Split { axis: Axis, kids: Vec<Node> },
}

impl Node {
    pub fn leaves(&self, out: &mut Vec<usize>) {
        match self {
            Node::Leaf(i) => out.push(*i),
            Node::Split { kids, .. } => {
                for k in kids {
                    k.leaves(out);
                }
            }
        }
    }

    pub fn split_leaf(&mut self, leaf: usize, axis: Axis, new: usize) -> bool {
        match self {
            Node::Leaf(i) => {
                if *i != leaf {
                    return false;
                }
                *self = Node::Split {
                    axis,
                    kids: vec![Node::Leaf(leaf), Node::Leaf(new)],
                };
                true
            }
            Node::Split { axis: a, kids } => {
                let mut done = false;
                for k in kids.iter_mut() {
                    if k.split_leaf(leaf, axis, new) {
                        done = true;
                        break;
                    }
                }
                if done && *a == axis {
                    let mut flat: Vec<Node> = Vec::with_capacity(kids.len());
                    for k in kids.drain(..) {
                        match k {
                            Node::Split { axis: ka, kids: kk } if ka == *a => flat.extend(kk),
                            other => flat.push(other),
                        }
                    }
                    *kids = flat;
                }
                done
            }
        }
    }

    pub fn remove_leaf(&mut self, leaf: usize) -> bool {
        match self {
            Node::Leaf(i) => *i == leaf,
            Node::Split { kids, .. } => {
                let mut removed = false;
                for pos in 0..kids.len() {
                    if kids[pos].remove_leaf(leaf) {
                        kids.remove(pos);
                        removed = true;
                        break;
                    }
                }
                if removed && kids.len() == 1 {
                    *self = kids.remove(0);
                }
                removed
            }
        }
    }

    pub fn drop_id(&mut self, id: usize) {
        match self {
            Node::Leaf(i) => {
                if *i > id {
                    *i -= 1;
                }
            }
            Node::Split { kids, .. } => {
                for k in kids {
                    k.drop_id(id);
                }
            }
        }
    }

    pub fn rects(&self, r: Rect, out: &mut Vec<(usize, Rect)>) {
        match self {
            Node::Leaf(i) => out.push((*i, r)),
            Node::Split { axis, kids } => {
                let n = kids.len() as u32;
                if n == 0 {
                    return;
                }
                let (total, cross) = match axis {
                    Axis::Columns => (r.cols, r.rows),
                    Axis::Rows => (r.rows, r.cols),
                };
                let base = total / n;
                let rem = total % n;
                let mut at = match axis {
                    Axis::Columns => r.x,
                    Axis::Rows => r.y,
                };
                for (i, k) in kids.iter().enumerate() {
                    let span = base + if (i as u32) < rem { 1 } else { 0 };
                    let sub = match axis {
                        Axis::Columns => Rect {
                            x: at,
                            y: r.y,
                            cols: span,
                            rows: cross,
                        },
                        Axis::Rows => Rect {
                            x: r.x,
                            y: at,
                            cols: cross,
                            rows: span,
                        },
                    };
                    k.rects(sub, out);
                    at += span;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(t: &Node) -> Vec<usize> {
        let mut v = Vec::new();
        t.leaves(&mut v);
        v
    }

    fn rects(t: &Node, cols: u32, rows: u32) -> Vec<(usize, Rect)> {
        let mut v = Vec::new();
        t.rects(
            Rect {
                x: 0,
                y: 0,
                cols,
                rows,
            },
            &mut v,
        );
        v
    }

    #[test]
    fn split_makes_two_leaves_side_by_side() {
        let mut t = Node::Leaf(0);
        assert!(t.split_leaf(0, Axis::Columns, 1));
        assert_eq!(order(&t), vec![0, 1]);
        assert_eq!(
            rects(&t, 101, 40),
            vec![
                (
                    0,
                    Rect {
                        x: 0,
                        y: 0,
                        cols: 51,
                        rows: 40
                    }
                ),
                (
                    1,
                    Rect {
                        x: 51,
                        y: 0,
                        cols: 50,
                        rows: 40
                    }
                ),
            ]
        );
    }

    #[test]
    fn same_axis_splits_stay_in_one_row() {
        let mut t = Node::Leaf(0);
        t.split_leaf(0, Axis::Columns, 1);
        t.split_leaf(1, Axis::Columns, 2);
        assert_eq!(order(&t), vec![0, 1, 2]);
        let r = rects(&t, 90, 40);
        assert_eq!(
            r,
            vec![
                (
                    0,
                    Rect {
                        x: 0,
                        y: 0,
                        cols: 30,
                        rows: 40
                    }
                ),
                (
                    1,
                    Rect {
                        x: 30,
                        y: 0,
                        cols: 30,
                        rows: 40
                    }
                ),
                (
                    2,
                    Rect {
                        x: 60,
                        y: 0,
                        cols: 30,
                        rows: 40
                    }
                ),
            ]
        );
    }

    #[test]
    fn other_axis_splits_nest() {
        let mut t = Node::Leaf(0);
        t.split_leaf(0, Axis::Columns, 1);
        t.split_leaf(1, Axis::Rows, 2);
        assert_eq!(order(&t), vec![0, 1, 2]);
        assert_eq!(
            rects(&t, 100, 40),
            vec![
                (
                    0,
                    Rect {
                        x: 0,
                        y: 0,
                        cols: 50,
                        rows: 40
                    }
                ),
                (
                    1,
                    Rect {
                        x: 50,
                        y: 0,
                        cols: 50,
                        rows: 20
                    }
                ),
                (
                    2,
                    Rect {
                        x: 50,
                        y: 20,
                        cols: 50,
                        rows: 20
                    }
                ),
            ]
        );
    }

    #[test]
    fn split_only_touches_the_named_leaf() {
        let mut t = Node::Leaf(0);
        t.split_leaf(0, Axis::Columns, 1);
        assert!(!t.split_leaf(7, Axis::Rows, 9));
        assert_eq!(order(&t), vec![0, 1]);
    }

    #[test]
    fn removing_a_leaf_collapses_the_split() {
        let mut t = Node::Leaf(0);
        t.split_leaf(0, Axis::Rows, 1);
        assert!(t.remove_leaf(1));
        assert_eq!(order(&t), vec![0]);
        assert_eq!(t, Node::Leaf(0));
        assert!(!t.remove_leaf(1));
    }

    #[test]
    fn removing_the_middle_leaf_keeps_the_siblings() {
        let mut t = Node::Leaf(0);
        t.split_leaf(0, Axis::Columns, 1);
        t.split_leaf(1, Axis::Columns, 2);
        assert!(t.remove_leaf(1));
        assert_eq!(
            rects(&t, 100, 10),
            vec![
                (
                    0,
                    Rect {
                        x: 0,
                        y: 0,
                        cols: 50,
                        rows: 10
                    }
                ),
                (
                    2,
                    Rect {
                        x: 50,
                        y: 0,
                        cols: 50,
                        rows: 10
                    }
                ),
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

    #[test]
    fn drop_id_shifts_later_leaves() {
        let mut t = Node::Leaf(0);
        t.split_leaf(0, Axis::Columns, 1);
        t.split_leaf(1, Axis::Rows, 2);
        assert_eq!(order(&t), vec![0, 1, 2]);
        assert!(t.remove_leaf(0));
        t.drop_id(0);
        assert_eq!(order(&t), vec![0, 1]);
        assert!(t.remove_leaf(0));
        assert_eq!(order(&t), vec![1]);
    }
}
