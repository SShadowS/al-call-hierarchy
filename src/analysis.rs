//! Code quality analysis - cyclomatic complexity and metrics

use serde::Serialize;
use tree_sitter::{Node, TreeCursor};

/// Metrics for a single procedure/trigger
#[derive(Debug, Clone, Serialize)]
pub struct ProcedureMetrics {
    pub object_type: String,
    pub object_name: String,
    pub procedure_name: String,
    pub file: String,
    pub line: u32,
    pub complexity: u32,
    pub line_count: u32,
    pub parameter_count: u32,
    pub quality_score: f32,
}

/// A finding/issue detected during analysis
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub category: String,
    pub severity: String,
    pub location: String,
    pub procedure: String,
    pub description: String,
}

/// Summary statistics for the analysis
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisSummary {
    pub total_procedures: usize,
    pub avg_complexity: f32,
    pub avg_quality_score: f32,
    pub critical_findings: usize,
    pub warning_findings: usize,
}

/// Complete analysis result
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    pub metrics: Vec<ProcedureMetrics>,
    pub findings: Vec<Finding>,
    pub summary: AnalysisSummary,
}

use crate::config::DiagnosticConfig;

/// Calculate cyclomatic complexity by walking the AST subtree
///
/// Counts decision points:
/// - if statements (+1 for if, +1 for else)
/// - case branches (+1 per branch)
/// - loops: while, for, foreach, repeat (+1 each)
/// - logical operators: and, or (+1 each)
pub fn calculate_complexity(node: &Node) -> u32 {
    let mut complexity = 1; // Base complexity (single path)
    let mut cursor = node.walk();
    count_decision_points(&mut cursor, &mut complexity);
    complexity
}

fn count_decision_points(cursor: &mut TreeCursor, complexity: &mut u32) {
    let node = cursor.node();
    let kind = node.kind();

    match kind {
        // Control flow statements
        "if_statement" => {
            *complexity += 1;
            // Check if there's an else branch
            if node.child_by_field_name("else_branch").is_some() {
                *complexity += 1;
            }
        }
        "while_statement" | "for_statement" | "foreach_statement" | "repeat_statement" => {
            *complexity += 1;
        }
        // Case branches (each branch adds a path)
        "case_branch" => {
            *complexity += 1;
        }
        // Logical operators add decision points
        "logical_expression" => {
            // Check the operator field for and/or
            if let Some(op_node) = node.child_by_field_name("operator") {
                let op = op_node.kind().to_lowercase();
                if op == "and" || op == "or" {
                    *complexity += 1;
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    if cursor.goto_first_child() {
        loop {
            count_decision_points(cursor, complexity);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// Calculate quality score on a 0-10 scale
/// Based on tree-sitter-mcp's quality score formula
pub fn calculate_quality_score(complexity: u32, line_count: u32, params: u32) -> f32 {
    let mut score = 10.0;

    // Complexity penalties
    if complexity > 4 {
        score -= 1.6 + (complexity - 4) as f32 * 1.2;
    } else if complexity > 2 {
        score -= (complexity - 2) as f32 * 0.8;
    }

    // Method length penalties
    if line_count > 15 {
        score -= 1.5 + (line_count - 15) as f32 * 0.15;
    } else if line_count > 10 {
        score -= (line_count - 10) as f32 * 0.3;
    }

    // Parameter penalties
    if params > 4 {
        score -= 1.0 + (params - 4) as f32 * 0.8;
    } else if params > 2 {
        score -= (params - 2) as f32 * 0.5;
    }

    score.max(0.0).min(10.0)
}

/// Generate findings based on metrics
pub fn generate_findings(metrics: &ProcedureMetrics, config: &DiagnosticConfig) -> Vec<Finding> {
    let mut findings = Vec::new();
    let location = format!("{}:{}", metrics.file, metrics.line);
    let procedure = format!("{}.{}", metrics.object_name, metrics.procedure_name);

    // Complexity findings
    if config.complexity_enabled && metrics.complexity >= config.complexity_critical {
        findings.push(Finding {
            category: "high_complexity".to_string(),
            severity: "critical".to_string(),
            location: location.clone(),
            procedure: procedure.clone(),
            description: format!(
                "Cyclomatic complexity {} exceeds critical threshold of {}",
                metrics.complexity, config.complexity_critical
            ),
        });
    } else if config.complexity_enabled && metrics.complexity >= config.complexity_warning {
        findings.push(Finding {
            category: "high_complexity".to_string(),
            severity: "warning".to_string(),
            location: location.clone(),
            procedure: procedure.clone(),
            description: format!(
                "Cyclomatic complexity {} exceeds warning threshold of {}",
                metrics.complexity, config.complexity_warning
            ),
        });
    }

    // Length findings
    if config.length_enabled && metrics.line_count >= config.length_critical {
        findings.push(Finding {
            category: "long_method".to_string(),
            severity: "critical".to_string(),
            location: location.clone(),
            procedure: procedure.clone(),
            description: format!(
                "Method length {} lines exceeds critical threshold of {}",
                metrics.line_count, config.length_critical
            ),
        });
    } else if config.length_enabled && metrics.line_count >= config.length_warning {
        findings.push(Finding {
            category: "long_method".to_string(),
            severity: "warning".to_string(),
            location: location.clone(),
            procedure: procedure.clone(),
            description: format!(
                "Method length {} lines exceeds warning threshold of {}",
                metrics.line_count, config.length_warning
            ),
        });
    }

    // Parameter findings
    if config.params_enabled && metrics.parameter_count >= config.params_critical {
        findings.push(Finding {
            category: "too_many_parameters".to_string(),
            severity: "critical".to_string(),
            location: location.clone(),
            procedure: procedure.clone(),
            description: format!(
                "Parameter count {} exceeds critical threshold of {}",
                metrics.parameter_count, config.params_critical
            ),
        });
    } else if config.params_enabled && metrics.parameter_count >= config.params_warning {
        findings.push(Finding {
            category: "too_many_parameters".to_string(),
            severity: "warning".to_string(),
            location,
            procedure,
            description: format!(
                "Parameter count {} exceeds warning threshold of {}",
                metrics.parameter_count, config.params_warning
            ),
        });
    }

    findings
}

/// Build analysis summary from metrics and findings
pub fn build_summary(metrics: &[ProcedureMetrics], findings: &[Finding]) -> AnalysisSummary {
    let total = metrics.len();
    let avg_complexity = if total > 0 {
        metrics.iter().map(|m| m.complexity as f32).sum::<f32>() / total as f32
    } else {
        0.0
    };
    let avg_quality = if total > 0 {
        metrics.iter().map(|m| m.quality_score).sum::<f32>() / total as f32
    } else {
        0.0
    };

    let critical = findings.iter().filter(|f| f.severity == "critical").count();
    let warnings = findings.iter().filter(|f| f.severity == "warning").count();

    AnalysisSummary {
        total_procedures: total,
        avg_complexity,
        avg_quality_score: avg_quality,
        critical_findings: critical,
        warning_findings: warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to find procedure nodes in the AST for testing
    fn find_procedure_node<'a>(
        cursor: &mut tree_sitter::TreeCursor<'a>,
        name: &str,
        source: &str,
    ) -> Option<tree_sitter::Node<'a>> {
        let node = cursor.node();
        if node.kind() == "procedure" {
            // Check if this procedure has the name we're looking for
            // The field is named "name" in the grammar
            if let Some(name_node) = node.child_by_field_name("name") {
                let proc_name = &source[name_node.byte_range()];
                if proc_name == name {
                    return Some(node);
                }
            }
        }

        if cursor.goto_first_child() {
            loop {
                if let Some(found) = find_procedure_node(cursor, name, source) {
                    cursor.goto_parent();
                    return Some(found);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
        None
    }

    #[test]
    fn test_complexity_calculation_with_actual_al_parsing() {
        // Test AL code with various control flow constructs
        // Expected complexity calculation:
        // Base: 1
        // if i > 0: +1 (no else)
        // nested if i > 10: +1, else: +1 (has else branch)
        // while not done: +1
        // if (i > 0) and (i < 100): +1 for if, +1 for and
        // case: 2 case_branch items (+2, case_else_branch is not counted)
        // Total expected: 1 + 1 + 2 + 1 + 2 + 2 = 9
        //
        // Note: case_else_branch is correctly NOT counted per standard cyclomatic complexity
        // (the else/default path is not a decision point)
        let al_code = r#"codeunit 50100 "Test"
{
    procedure ComplexProcedure()
    var
        i: Integer;
        done: Boolean;
    begin
        if i > 0 then begin
            if i > 10 then
                i := 10
            else
                i := 5;
        end;

        while not done do begin
            if (i > 0) and (i < 100) then
                i += 1;
            done := i >= 100;
        end;

        case i of
            1: i := 2;
            2: i := 3;
            else i := 0;
        end;
    end;
}"#;

        // Parse the AL code
        let lang = crate::language::language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("Failed to set language");
        let tree = parser.parse(al_code, None).expect("Failed to parse");

        // Find the ComplexProcedure node
        let mut cursor = tree.walk();
        let proc_node = find_procedure_node(&mut cursor, "ComplexProcedure", al_code)
            .expect("Could not find ComplexProcedure");

        // Calculate complexity
        let complexity = calculate_complexity(&proc_node);

        // Expected 9 (1 + if + nested if/else + while + if + `and` + 2 case branches).
        // The tree-sitter-al grammar fix (owned-IR consumer: named in/is/as expressions,
        // removing the spurious left/operator/right field bleed) corrected a previous
        // undercount to 8 — the clean grammar now yields the theoretically correct 9.
        assert_eq!(complexity, 9, "Expected complexity 9, got {}", complexity);
    }

    #[test]
    fn test_complexity_if_else() {
        // Test if-else counting: if adds +1, else adds +1
        let al_code = r#"codeunit 50100 "Test"
{
    procedure IfElseProcedure()
    var
        i: Integer;
    begin
        if i > 0 then
            i := 1
        else
            i := 0;
    end;
}"#;

        let lang = crate::language::language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("Failed to set language");
        let tree = parser.parse(al_code, None).expect("Failed to parse");

        let mut cursor = tree.walk();
        let proc_node = find_procedure_node(&mut cursor, "IfElseProcedure", al_code)
            .expect("Could not find IfElseProcedure");

        let complexity = calculate_complexity(&proc_node);
        // Base: 1, if: +1, else: +1 = 3
        assert_eq!(
            complexity, 3,
            "If-else procedure should have complexity 3, got {}",
            complexity
        );
    }

    #[test]
    fn test_complexity_simple_procedure() {
        // A simple procedure with no control flow should have complexity 1
        let al_code = r#"codeunit 50100 "Test"
{
    procedure SimpleProcedure()
    begin
        Message('Hello');
    end;
}"#;

        let lang = crate::language::language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("Failed to set language");
        let tree = parser.parse(al_code, None).expect("Failed to parse");

        let mut cursor = tree.walk();
        let proc_node = find_procedure_node(&mut cursor, "SimpleProcedure", al_code)
            .expect("Could not find SimpleProcedure");

        let complexity = calculate_complexity(&proc_node);
        assert_eq!(complexity, 1, "Simple procedure should have complexity 1");
    }

    #[test]
    fn test_complexity_loops() {
        // Test all loop types: for, foreach, repeat, while
        let al_code = r#"codeunit 50100 "Test"
{
    procedure LoopProcedure()
    var
        i: Integer;
        items: List of [Integer];
    begin
        for i := 1 to 10 do
            i := i;

        foreach i in items do
            i := i;

        repeat
            i += 1;
        until i > 10;

        while i < 20 do
            i += 1;
    end;
}"#;

        let lang = crate::language::language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("Failed to set language");
        let tree = parser.parse(al_code, None).expect("Failed to parse");

        let mut cursor = tree.walk();
        let proc_node = find_procedure_node(&mut cursor, "LoopProcedure", al_code)
            .expect("Could not find LoopProcedure");

        let complexity = calculate_complexity(&proc_node);
        // Base: 1, for: +1, foreach: +1, repeat: +1, while: +1 = 5
        assert_eq!(
            complexity, 5,
            "Loop procedure should have complexity 5, got {}",
            complexity
        );
    }

    #[test]
    fn test_complexity_logical_operators() {
        // Test logical operators: and, or (xor is not counted per standard CC)
        let al_code = r#"codeunit 50100 "Test"
{
    procedure LogicalProcedure()
    var
        a: Boolean;
        b: Boolean;
        c: Boolean;
    begin
        if a and b then
            c := true;

        if a or b then
            c := false;

        if a and b and c then
            c := true;
    end;
}"#;

        let lang = crate::language::language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).expect("Failed to set language");
        let tree = parser.parse(al_code, None).expect("Failed to parse");

        let mut cursor = tree.walk();
        let proc_node = find_procedure_node(&mut cursor, "LogicalProcedure", al_code)
            .expect("Could not find LogicalProcedure");

        let complexity = calculate_complexity(&proc_node);
        // Base: 1
        // if a and b: +1 (if) +1 (and)
        // if a or b: +1 (if) +1 (or)
        // if a and b and c: +1 (if) +2 (two and operators)
        // Total: 1 + 2 + 2 + 3 = 8
        assert_eq!(
            complexity, 8,
            "Logical procedure should have complexity 8, got {}",
            complexity
        );
    }

    #[test]
    fn test_quality_score_perfect() {
        let score = calculate_quality_score(1, 5, 1);
        assert!((score - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_quality_score_high_complexity() {
        let score = calculate_quality_score(10, 5, 1);
        assert!(score < 5.0);
    }

    #[test]
    fn test_quality_score_long_method() {
        let score = calculate_quality_score(1, 50, 1);
        assert!(score < 5.0);
    }

    #[test]
    fn test_quality_score_many_params() {
        let score = calculate_quality_score(1, 5, 8);
        assert!(score < 7.0);
    }

    #[test]
    fn test_findings_generated_for_high_complexity() {
        let metrics = ProcedureMetrics {
            object_type: "Codeunit".to_string(),
            object_name: "Test".to_string(),
            procedure_name: "TestProc".to_string(),
            file: "test.al".to_string(),
            line: 10,
            complexity: 12,
            line_count: 10,
            parameter_count: 2,
            quality_score: 5.0,
        };
        let config = DiagnosticConfig::default();
        let findings = generate_findings(&metrics, &config);
        assert!(findings
            .iter()
            .any(|f| f.category == "high_complexity" && f.severity == "critical"));
    }

    #[test]
    fn test_quality_score_moderate_complexity() {
        // complexity 3 hits the else-if branch (> 2 but <= 4)
        let score = calculate_quality_score(3, 5, 1);
        // penalty = (3-2) * 0.8 = 0.8, so score = 9.2
        assert!((score - 9.2).abs() < 0.01);
    }

    #[test]
    fn test_quality_score_moderate_length() {
        // line_count 12 hits the else-if branch (> 10 but <= 15)
        let score = calculate_quality_score(1, 12, 1);
        // penalty = (12-10) * 0.3 = 0.6, so score = 9.4
        assert!((score - 9.4).abs() < 0.01);
    }

    #[test]
    fn test_quality_score_moderate_params() {
        // params 3 hits the else-if branch (> 2 but <= 4)
        let score = calculate_quality_score(1, 5, 3);
        // penalty = (3-2) * 0.5 = 0.5, so score = 9.5
        assert!((score - 9.5).abs() < 0.01);
    }

    #[test]
    fn test_quality_score_clamped_to_zero() {
        // Very bad metrics should clamp to 0
        let score = calculate_quality_score(50, 200, 20);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_findings_complexity_warning() {
        let config = DiagnosticConfig::default();
        let metrics = ProcedureMetrics {
            object_type: "Codeunit".to_string(),
            object_name: "Test".to_string(),
            procedure_name: "TestProc".to_string(),
            file: "test.al".to_string(),
            line: 10,
            complexity: config.complexity_warning, // at warning threshold
            line_count: 5,
            parameter_count: 1,
            quality_score: 8.0,
        };
        let findings = generate_findings(&metrics, &config);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "high_complexity");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn test_findings_length_critical() {
        let config = DiagnosticConfig::default();
        let metrics = ProcedureMetrics {
            object_type: "Codeunit".to_string(),
            object_name: "Test".to_string(),
            procedure_name: "TestProc".to_string(),
            file: "test.al".to_string(),
            line: 10,
            complexity: 1,
            line_count: config.length_critical, // at critical threshold
            parameter_count: 1,
            quality_score: 5.0,
        };
        let findings = generate_findings(&metrics, &config);
        assert!(findings
            .iter()
            .any(|f| f.category == "long_method" && f.severity == "critical"));
    }

    #[test]
    fn test_findings_length_warning() {
        let config = DiagnosticConfig::default();
        let metrics = ProcedureMetrics {
            object_type: "Codeunit".to_string(),
            object_name: "Test".to_string(),
            procedure_name: "TestProc".to_string(),
            file: "test.al".to_string(),
            line: 10,
            complexity: 1,
            line_count: config.length_warning, // at warning threshold
            parameter_count: 1,
            quality_score: 7.0,
        };
        let findings = generate_findings(&metrics, &config);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "long_method");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn test_findings_params_critical() {
        let config = DiagnosticConfig::default();
        let metrics = ProcedureMetrics {
            object_type: "Codeunit".to_string(),
            object_name: "Test".to_string(),
            procedure_name: "TestProc".to_string(),
            file: "test.al".to_string(),
            line: 10,
            complexity: 1,
            line_count: 5,
            parameter_count: config.params_critical, // at critical threshold
            quality_score: 5.0,
        };
        let findings = generate_findings(&metrics, &config);
        assert!(findings
            .iter()
            .any(|f| f.category == "too_many_parameters" && f.severity == "critical"));
    }

    #[test]
    fn test_findings_params_warning() {
        let config = DiagnosticConfig::default();
        let metrics = ProcedureMetrics {
            object_type: "Codeunit".to_string(),
            object_name: "Test".to_string(),
            procedure_name: "TestProc".to_string(),
            file: "test.al".to_string(),
            line: 10,
            complexity: 1,
            line_count: 5,
            parameter_count: config.params_warning, // at warning threshold
            quality_score: 7.0,
        };
        let findings = generate_findings(&metrics, &config);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "too_many_parameters");
        assert_eq!(findings[0].severity, "warning");
    }

    #[test]
    fn test_findings_no_issues() {
        let config = DiagnosticConfig::default();
        let metrics = ProcedureMetrics {
            object_type: "Codeunit".to_string(),
            object_name: "Test".to_string(),
            procedure_name: "TestProc".to_string(),
            file: "test.al".to_string(),
            line: 10,
            complexity: 1,
            line_count: 5,
            parameter_count: 1,
            quality_score: 10.0,
        };
        let findings = generate_findings(&metrics, &config);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_build_summary_with_metrics() {
        let metrics = vec![
            ProcedureMetrics {
                object_type: "Codeunit".to_string(),
                object_name: "Test".to_string(),
                procedure_name: "Proc1".to_string(),
                file: "test.al".to_string(),
                line: 10,
                complexity: 4,
                line_count: 20,
                parameter_count: 2,
                quality_score: 8.0,
            },
            ProcedureMetrics {
                object_type: "Codeunit".to_string(),
                object_name: "Test".to_string(),
                procedure_name: "Proc2".to_string(),
                file: "test.al".to_string(),
                line: 30,
                complexity: 6,
                line_count: 30,
                parameter_count: 3,
                quality_score: 6.0,
            },
        ];
        let findings = vec![
            Finding {
                category: "high_complexity".to_string(),
                severity: "critical".to_string(),
                location: "test.al:30".to_string(),
                procedure: "Test.Proc2".to_string(),
                description: "test".to_string(),
            },
            Finding {
                category: "long_method".to_string(),
                severity: "warning".to_string(),
                location: "test.al:30".to_string(),
                procedure: "Test.Proc2".to_string(),
                description: "test".to_string(),
            },
        ];
        let summary = build_summary(&metrics, &findings);
        assert_eq!(summary.total_procedures, 2);
        assert!((summary.avg_complexity - 5.0).abs() < 0.01);
        assert!((summary.avg_quality_score - 7.0).abs() < 0.01);
        assert_eq!(summary.critical_findings, 1);
        assert_eq!(summary.warning_findings, 1);
    }

    #[test]
    fn test_build_summary_empty() {
        let summary = build_summary(&[], &[]);
        assert_eq!(summary.total_procedures, 0);
        assert_eq!(summary.avg_complexity, 0.0);
        assert_eq!(summary.avg_quality_score, 0.0);
        assert_eq!(summary.critical_findings, 0);
        assert_eq!(summary.warning_findings, 0);
    }
}
