#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CmdKind {
    Ex,
    Search,
    SearchBack,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    Command(CmdKind),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FindDir {
    F,
    T,
    BackF,
    BackT,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pending {
    None,
    Op(Operator),
    G,
    Z,
    WinCmd,
    F { forward: bool, to: bool },
    R,
    TextObj { op: Operator, inner: bool },
    OpG { op: Operator },
}
