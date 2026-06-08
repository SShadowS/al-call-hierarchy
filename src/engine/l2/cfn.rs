//! Normalized CFN skeleton builder — port of the `buildCFN*` /
//! `harvestExpressionLeaves` closures in `intraprocedural-body.ts`.
//!
//! Built POST-visit: reads the `node.id`-keyed op/callsite maps populated during
//! the walk, so all ids are final. The projection DROPS each node's sourceAnchor
//! (skeleton = kind + child/else structure + op/callsite refs + conditionGuard +
//! ordered conditionLeaves only).

use super::features::{PCFNNode, PConditionGuard};
use super::node_util::{child_of_kind, named_children, node_text, Utf16Cols};
use std::collections::HashMap;
use tree_sitter::Node;

pub struct CfnCtx<'a> {
    pub source: &'a str,
    pub op_id_by_node_id: &'a HashMap<usize, String>,
    pub cs_id_by_node_id: &'a HashMap<usize, String>,
    /// utf16-column converter — used to stamp each CFN node's TRUE source range
    /// (in PAnchor basis) so the L4 branch-aware walker can attribute field
    /// accesses to the right block level. The range never serializes (L2 parity).
    pub cols: &'a Utf16Cols<'a>,
}

fn node(kind: &str) -> PCFNNode {
    PCFNNode {
        kind: kind.to_string(),
        operation_id: None,
        callsite_id: None,
        condition_guard: None,
        condition_leaves: None,
        children: None,
        else_children: None,
        is_case_else: false,
        source_range: None,
    }
}

impl<'a> CfnCtx<'a> {
    /// The (startLine, startColumn, endLine, endColumn) of a tree-sitter node in
    /// PAnchor basis (0-based row, utf16 column).
    fn range_of(&self, node: Node) -> (u32, u32, u32, u32) {
        let sp = node.start_position();
        let ep = node.end_position();
        (
            sp.row as u32,
            self.cols.col(sp.row, sp.column),
            ep.row as u32,
            self.cols.col(ep.row, ep.column),
        )
    }

    /// Build the root block CFN for a code_block.
    pub fn build_block(&self, block_node: Node) -> PCFNNode {
        let mut children = Vec::new();
        for child in named_children(block_node) {
            let t = child.kind();
            if t == "begin_keyword" || t == "end_keyword" {
                continue;
            }
            if let Some(cfn) = self.build_statement(child) {
                children.push(cfn);
            }
        }
        let mut n = node("block");
        n.children = Some(children);
        n.source_range = Some(self.range_of(block_node));
        n
    }

    fn build_branch_body(&self, node_in: Node) -> PCFNNode {
        if node_in.kind() == "code_block" {
            return self.build_block(node_in);
        }
        let stmt = self.build_statement(node_in);
        let mut n = node("block");
        n.children = Some(stmt.into_iter().collect());
        n.source_range = Some(self.range_of(node_in));
        n
    }

    /// Harvest receiver-side leaves of a chained call into `out`.
    fn harvest_receiver_leaves(&self, call_node: Node, out: &mut Vec<PCFNNode>) {
        let func_node = call_node
            .child_by_field_name("function")
            .or_else(|| call_node.named_child(0));
        let Some(func_node) = func_node else {
            return;
        };
        if func_node.kind() != "member_expression" {
            return;
        }
        let obj_node = func_node
            .child_by_field_name("object")
            .or_else(|| func_node.named_child(0));
        let Some(obj_node) = obj_node else {
            return;
        };
        if obj_node.kind() != "call_expression" && obj_node.kind() != "member_expression" {
            return;
        }
        self.harvest_expression_leaves(Some(obj_node), out);
    }

    /// Harvest op/callsite leaves from an expression subtree.
    fn harvest_expression_leaves(&self, expr_node: Option<Node>, out: &mut Vec<PCFNNode>) {
        let Some(expr_node) = expr_node else {
            return;
        };
        let t = expr_node.kind();
        if t == "call_expression" || t == "member_expression" {
            if let Some(op_id) = self.op_id_by_node_id.get(&expr_node.id()) {
                let mut inner = Vec::new();
                if let Some(arg_list) = child_of_kind(expr_node, "argument_list") {
                    for arg in named_children(arg_list) {
                        self.harvest_expression_leaves(Some(arg), &mut inner);
                    }
                }
                self.harvest_receiver_leaves(expr_node, out);
                let mut leaf = node("op");
                leaf.operation_id = Some(op_id.clone());
                if !inner.is_empty() {
                    leaf.condition_leaves = Some(inner);
                }
                out.push(leaf);
                return;
            }
            if let Some(cs_id) = self.cs_id_by_node_id.get(&expr_node.id()) {
                let mut inner = Vec::new();
                if let Some(arg_list) = child_of_kind(expr_node, "argument_list") {
                    for arg in named_children(arg_list) {
                        self.harvest_expression_leaves(Some(arg), &mut inner);
                    }
                }
                let func_node = expr_node
                    .child_by_field_name("function")
                    .or_else(|| expr_node.named_child(0));
                self.harvest_receiver_leaves(expr_node, out);
                let is_error = func_node.is_some_and(|f| {
                    f.kind() == "identifier" && node_text(f, self.source).to_lowercase() == "error"
                });
                let mut leaf = node(if is_error { "error" } else { "call" });
                leaf.callsite_id = Some(cs_id.clone());
                if !inner.is_empty() {
                    leaf.condition_leaves = Some(inner);
                }
                out.push(leaf);
                return;
            }
        }
        for child in named_children(expr_node) {
            self.harvest_expression_leaves(Some(child), out);
        }
    }

    /// Recognize a simple boolean-guard condition.
    fn simple_condition_guard(&self, condition: Option<Node>) -> Option<PConditionGuard> {
        let condition = condition?;
        if condition.kind() == "identifier" {
            return Some(PConditionGuard {
                identifier: node_text(condition, self.source).to_lowercase(),
                polarity: "positive".to_string(),
            });
        }
        if condition.kind() == "unary_expression" {
            let op = condition.child_by_field_name("operator")?;
            if node_text(op, self.source).to_lowercase() != "not" {
                return None;
            }
            let operand = named_children(condition)
                .into_iter()
                .find(|c| c.kind() == "identifier")?;
            return Some(PConditionGuard {
                identifier: node_text(operand, self.source).to_lowercase(),
                polarity: "negative".to_string(),
            });
        }
        if condition.kind() == "comparison_expression" {
            let left = condition.child_by_field_name("left")?;
            let operator = condition.child_by_field_name("operator")?;
            let right = condition.child_by_field_name("right")?;
            if node_text(operator, self.source) != "=" {
                return None;
            }
            let id_side = if left.kind() == "identifier" {
                Some(left)
            } else if right.kind() == "identifier" {
                Some(right)
            } else {
                None
            };
            let lit_side = if left.kind() == "boolean" {
                Some(left)
            } else if right.kind() == "boolean" {
                Some(right)
            } else {
                None
            };
            let (id_side, lit_side) = (id_side?, lit_side?);
            if node_text(lit_side, self.source).to_lowercase() != "false" {
                return None;
            }
            return Some(PConditionGuard {
                identifier: node_text(id_side, self.source).to_lowercase(),
                polarity: "negative".to_string(),
            });
        }
        None
    }

    fn build_case_branch(&self, node_in: Node) -> Option<PCFNNode> {
        let mut body_node: Option<Node> = None;
        if node_in.kind() == "case_branch" {
            body_node = node_in.child_by_field_name("body");
        } else {
            // case_else_branch: prefer a code_block child, else first non-keyword named child.
            for child in named_children(node_in) {
                if child.kind() == "code_block" {
                    body_node = Some(child);
                    break;
                }
            }
            if body_node.is_none() {
                for child in named_children(node_in) {
                    if child.kind() == "else_keyword" {
                        continue;
                    }
                    body_node = Some(child);
                    break;
                }
            }
        }
        let children: Vec<PCFNNode> = body_node
            .map(|b| vec![self.build_branch_body(b)])
            .unwrap_or_default();
        let mut n = node("case-branch");
        n.children = Some(children);
        n.source_range = Some(self.range_of(node_in));
        // In-memory marker (never serialized) so the L4 branch-aware walker can
        // apply al-sem's "case WITHOUT else joins the pre-state" rule. Mirrors
        // al-sem's `sourceAnchor.syntaxKind === "case_else_branch"` check.
        n.is_case_else = node_in.kind() == "case_else_branch";
        Some(n)
    }

    /// Build a CFN node for a single statement node. None for skipped nodes.
    /// Stamps the node's TRUE source range (for L4 FA attribution).
    pub fn build_statement(&self, node_in: Node) -> Option<PCFNNode> {
        let mut cfn = self.build_statement_inner(node_in)?;
        if cfn.source_range.is_none() {
            cfn.source_range = Some(self.range_of(node_in));
        }
        Some(cfn)
    }

    fn build_statement_inner(&self, node_in: Node) -> Option<PCFNNode> {
        let t = node_in.kind();

        if t == "if_statement" {
            let then_branch = node_in.child_by_field_name("then_branch");
            let else_branch = node_in.child_by_field_name("else_branch");
            let children: Vec<PCFNNode> = then_branch
                .map(|b| vec![self.build_branch_body(b)])
                .unwrap_or_default();
            let else_children = else_branch.map(|b| vec![self.build_branch_body(b)]);
            let mut condition_leaves = Vec::new();
            let condition_node = node_in.child_by_field_name("condition");
            self.harvest_expression_leaves(condition_node, &mut condition_leaves);
            let mut result = node("if");
            result.children = Some(children);
            result.else_children = else_children;
            if !condition_leaves.is_empty() {
                result.condition_leaves = Some(condition_leaves);
            }
            if let Some(guard) = self.simple_condition_guard(condition_node) {
                result.condition_guard = Some(guard);
            }
            return Some(result);
        }

        if t == "case_statement" {
            let mut branch_cfns = Vec::new();
            for child in named_children(node_in) {
                if child.kind() == "case_branch" || child.kind() == "case_else_branch" {
                    if let Some(cfn) = self.build_case_branch(child) {
                        branch_cfns.push(cfn);
                    }
                }
            }
            let mut condition_leaves = Vec::new();
            self.harvest_expression_leaves(
                node_in.child_by_field_name("expression"),
                &mut condition_leaves,
            );
            let mut result = node("case");
            result.children = Some(branch_cfns);
            if !condition_leaves.is_empty() {
                result.condition_leaves = Some(condition_leaves);
            }
            return Some(result);
        }

        if t == "for_statement" || t == "foreach_statement" || t == "while_statement" {
            let kind = match t {
                "for_statement" => "for",
                "foreach_statement" => "foreach",
                _ => "while",
            };
            let body_node = node_in.child_by_field_name("body");
            let children: Vec<PCFNNode> = body_node
                .map(|b| vec![self.build_branch_body(b)])
                .unwrap_or_default();
            let mut condition_leaves = Vec::new();
            if t == "while_statement" {
                self.harvest_expression_leaves(
                    node_in.child_by_field_name("condition"),
                    &mut condition_leaves,
                );
            } else if t == "for_statement" {
                self.harvest_expression_leaves(
                    node_in.child_by_field_name("start"),
                    &mut condition_leaves,
                );
                self.harvest_expression_leaves(
                    node_in.child_by_field_name("end"),
                    &mut condition_leaves,
                );
            } else {
                self.harvest_expression_leaves(
                    node_in.child_by_field_name("iterable"),
                    &mut condition_leaves,
                );
            }
            let mut result = node(kind);
            result.children = Some(children);
            if !condition_leaves.is_empty() {
                result.condition_leaves = Some(condition_leaves);
            }
            return Some(result);
        }

        if t == "repeat_statement" {
            let condition_node = node_in.child_by_field_name("condition");
            let condition_start = condition_node.map(|c| c.start_byte());
            let mut body_children = Vec::new();
            for child in named_children(node_in) {
                let ct = child.kind();
                if ct == "until_keyword" || ct == "repeat_keyword" {
                    continue;
                }
                if Some(child.start_byte()) == condition_start {
                    break;
                }
                if let Some(cfn) = self.build_statement(child) {
                    body_children.push(cfn);
                }
            }
            let mut condition_leaves = Vec::new();
            self.harvest_expression_leaves(condition_node, &mut condition_leaves);
            let mut result = node("repeat");
            result.children = Some(body_children);
            if !condition_leaves.is_empty() {
                result.condition_leaves = Some(condition_leaves);
            }
            return Some(result);
        }

        if t == "try_statement" {
            let mut n = node("try");
            n.children = Some(vec![]);
            return Some(n);
        }

        if t == "exit_statement" {
            let mut exit_leaves = Vec::new();
            for child in named_children(node_in) {
                self.harvest_expression_leaves(Some(child), &mut exit_leaves);
            }
            let mut n = node("exit");
            if !exit_leaves.is_empty() {
                n.condition_leaves = Some(exit_leaves);
            }
            return Some(n);
        }

        if t == "identifier" {
            if let Some(op_id) = self.op_id_by_node_id.get(&node_in.id()) {
                let mut n = node("op");
                n.operation_id = Some(op_id.clone());
                return Some(n);
            }
            if let Some(cs_id) = self.cs_id_by_node_id.get(&node_in.id()) {
                let mut n = node("call");
                n.callsite_id = Some(cs_id.clone());
                return Some(n);
            }
            return None;
        }

        if t == "call_expression" || t == "member_expression" {
            let arg_list = child_of_kind(node_in, "argument_list");
            let mut pre_leaves = Vec::new();
            self.harvest_receiver_leaves(node_in, &mut pre_leaves);
            if let Some(arg_list) = arg_list {
                for arg in named_children(arg_list) {
                    self.harvest_expression_leaves(Some(arg), &mut pre_leaves);
                }
            }
            if let Some(op_id) = self.op_id_by_node_id.get(&node_in.id()) {
                let mut leaf = node("op");
                leaf.operation_id = Some(op_id.clone());
                if !pre_leaves.is_empty() {
                    leaf.condition_leaves = Some(pre_leaves);
                }
                return Some(leaf);
            }
            if let Some(cs_id) = self.cs_id_by_node_id.get(&node_in.id()) {
                let func_node = node_in
                    .child_by_field_name("function")
                    .or_else(|| node_in.named_child(0));
                let is_error = func_node.is_some_and(|f| {
                    f.kind() == "identifier" && node_text(f, self.source).to_lowercase() == "error"
                });
                let mut leaf = node(if is_error { "error" } else { "call" });
                leaf.callsite_id = Some(cs_id.clone());
                if !pre_leaves.is_empty() {
                    leaf.condition_leaves = Some(pre_leaves);
                }
                return Some(leaf);
            }
            let mut n = node("other");
            if !pre_leaves.is_empty() {
                n.condition_leaves = Some(pre_leaves);
            }
            return Some(n);
        }

        if t == "with_statement" || t == "asserterror_statement" {
            let body_node = node_in.child_by_field_name("body");
            if let Some(body_node) = body_node {
                let mut n = node("other");
                n.children = Some(vec![self.build_branch_body(body_node)]);
                return Some(n);
            }
            return Some(node("other"));
        }

        if matches!(
            t,
            "begin_keyword"
                | "end_keyword"
                | "if_keyword"
                | "then_keyword"
                | "else_keyword"
                | "case_keyword"
                | "of_keyword"
                | "repeat_keyword"
                | "until_keyword"
                | "while_keyword"
                | "for_keyword"
                | "do_keyword"
                | "foreach_keyword"
                | "in_keyword"
                | "empty_statement"
        ) {
            return None;
        }

        if t == "assignment_statement" {
            let mut rhs_leaves = Vec::new();
            for child in named_children(node_in) {
                self.harvest_expression_leaves(Some(child), &mut rhs_leaves);
            }
            let mut n = node("other");
            if !rhs_leaves.is_empty() {
                n.condition_leaves = Some(rhs_leaves);
            }
            return Some(n);
        }

        Some(node("other"))
    }
}
