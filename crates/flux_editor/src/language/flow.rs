//! Control-flow reachability. Conservative: a construct only "diverges" (never
//! falls through to the next statement) when that is certain, so we only ever
//! flag code that is *definitely* unreachable.

use super::ast::{Block, Stmt};

/// Does executing `block` always transfer control away (return/break/continue on
/// every path), so the statement after it is unreachable?
pub fn block_diverges(block: &Block) -> bool {
    block.stmts.iter().any(stmt_diverges)
}

/// Does this single statement always transfer control away?
pub fn stmt_diverges(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { .. } | Stmt::Break { .. } | Stmt::Continue { .. } => true,
        Stmt::Do { body, .. } => block_diverges(body),
        Stmt::If { arms, else_block, .. } => {
            // Only if every branch (including a present `else`) diverges.
            else_block.as_ref().is_some_and(block_diverges)
                && arms.iter().all(|(_, b)| block_diverges(b))
        }
        // Loops may run zero times or break out; don't assume divergence.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::diagnostics::Diagnostics;
    use crate::language::parser::parse;

    fn block(src: &str) -> Block {
        let mut d = Diagnostics::new(src);
        parse(src, &mut d)
    }

    #[test]
    fn return_diverges() {
        let b = block("return 1");
        assert!(block_diverges(&b));
    }

    #[test]
    fn if_without_else_falls_through() {
        let b = block("if x then return end");
        assert!(!block_diverges(&b));
    }

    #[test]
    fn if_with_else_all_return_diverges() {
        let b = block("if x then return 1 else return 2 end");
        assert!(block_diverges(&b));
    }
}
