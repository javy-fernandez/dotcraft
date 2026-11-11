//! Estimated execution plans (`SET SHOWPLAN_XML ON`) flattened into an operator tree.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanNode {
    pub depth: usize,
    pub physical: String,
    pub logical: String,
    /// `[schema].[table].[index]` the operator reads, if any.
    pub object: Option<String>,
    pub est_rows: f64,
    pub subtree_cost: f64,
    /// This operator's own share of the statement cost, 0–100.
    pub cost_percent: f64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    pub statement: String,
    pub total_cost: f64,
    /// Pre-order: each node is followed by its inputs at `depth + 1`.
    pub nodes: Vec<PlanNode>,
    /// `Impact 87%: dbo.Orders (CustomerId) INCLUDE (Total)`.
    pub missing_indexes: Vec<String>,
}

fn num(node: roxmltree::Node, name: &str) -> f64 {
    node.attribute(name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

/// Elements under `node` up to (not into) nested `RelOp`s.
fn own<'a, 'i>(node: roxmltree::Node<'a, 'i>, out: &mut Vec<roxmltree::Node<'a, 'i>>) {
    for child in node.children().filter(|c| c.is_element()) {
        out.push(child);
        if child.tag_name().name() != "RelOp" { own(child, out); }
    }
}

fn warnings(node: roxmltree::Node) -> Vec<String> {
    let mut out = Vec::new();
    for w in node.children().filter(|c| c.has_tag_name("Warnings")) {
        for item in w.children().filter(|c| c.is_element()) {
            let detail = item.attribute("ConvertIssue").or(item.attribute("Expression")).unwrap_or("");
            out.push(if detail.is_empty() { item.tag_name().name().to_string() } else { format!("{}: {detail}", item.tag_name().name()) });
        }
        for (name, value) in w.attributes().map(|a| (a.name(), a.value())) {
            if value == "true" || value == "1" { out.push(name.to_string()); }
        }
    }
    out
}

fn walk(node: roxmltree::Node, depth: usize, total: f64, out: &mut Vec<PlanNode>) {
    let mut inside = Vec::new();
    own(node, &mut inside);
    let object = inside.iter().find(|n| n.has_tag_name("Object")).map(|o| {
        ["Schema", "Table", "Index"].iter().filter_map(|a| o.attribute(*a)).collect::<Vec<_>>().join(".")
    });
    let children: Vec<_> = inside.iter().filter(|n| n.has_tag_name("RelOp")).copied().collect();
    let subtree = num(node, "EstimatedTotalSubtreeCost");
    let own_cost = (subtree - children.iter().map(|c| num(*c, "EstimatedTotalSubtreeCost")).sum::<f64>()).max(0.0);
    let mut warns = warnings(node);
    warns.dedup();
    out.push(PlanNode {
        depth,
        physical: node.attribute("PhysicalOp").unwrap_or("").to_string(),
        logical: node.attribute("LogicalOp").unwrap_or("").to_string(),
        object: object.filter(|o| !o.is_empty()),
        est_rows: num(node, "EstimateRows"),
        subtree_cost: subtree,
        cost_percent: if total > 0.0 { own_cost / total * 100.0 } else { 0.0 },
        warnings: warns,
    });
    for child in children { walk(child, depth + 1, total, out); }
}

/// Parses the plan of the first statement in a showplan document.
pub fn parse(xml: &str) -> Result<Plan, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let stmt = doc.descendants().find(|n| n.has_tag_name("StmtSimple")).ok_or("the plan has no statement")?;
    let mut plan = Plan { statement: stmt.attribute("StatementText").unwrap_or("").trim().to_string(), total_cost: num(stmt, "StatementSubTreeCost"), ..Plan::default() };
    let query_plan = stmt.children().find(|n| n.has_tag_name("QueryPlan"));
    if let Some(qp) = query_plan {
        for group in qp.descendants().filter(|n| n.has_tag_name("MissingIndexGroup")) {
            let impact = num(group, "Impact");
            for index in group.children().filter(|n| n.has_tag_name("MissingIndex")) {
                let table = ["Schema", "Table"].iter().filter_map(|a| index.attribute(*a)).collect::<Vec<_>>().join(".");
                let cols = |usage: &[&str]| index.children().filter(|g| g.has_tag_name("ColumnGroup") && usage.contains(&g.attribute("Usage").unwrap_or("")))
                    .flat_map(|g| g.children().filter(|c| c.has_tag_name("Column")).filter_map(|c| c.attribute("Name")).map(str::to_string).collect::<Vec<_>>()).collect::<Vec<_>>().join(", ");
                let mut text = format!("{impact:.0}%: {table} ({})", cols(&["EQUALITY", "INEQUALITY"]));
                let include = cols(&["INCLUDE"]);
                if !include.is_empty() { text.push_str(&format!(" INCLUDE ({include})")); }
                plan.missing_indexes.push(text);
            }
        }
        if let Some(root) = qp.children().find(|n| n.has_tag_name("RelOp")) {
            let total = if plan.total_cost > 0.0 { plan.total_cost } else { num(root, "EstimatedTotalSubtreeCost") };
            walk(root, 0, total, &mut plan.nodes);
        }
    }
    Ok(plan)
}

#[cfg(test)]
pub(crate) const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-16"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan" Version="1.6" Build="16.0.1000.6">
 <BatchSequence><Batch><Statements>
  <StmtSimple StatementText="SELECT c.Name, SUM(o.Total) FROM dbo.Orders o JOIN dbo.Customers c ON c.Id = o.CustomerId GROUP BY c.Name" StatementId="1" StatementSubTreeCost="0.0500" StatementEstRows="2" StatementType="SELECT">
   <QueryPlan CachedPlanSize="32">
    <MissingIndexes>
     <MissingIndexGroup Impact="61.5">
      <MissingIndex Database="[Shop]" Schema="[dbo]" Table="[Orders]">
       <ColumnGroup Usage="EQUALITY"><Column Name="[CustomerId]" ColumnId="2"/></ColumnGroup>
       <ColumnGroup Usage="INCLUDE"><Column Name="[Total]" ColumnId="3"/></ColumnGroup>
      </MissingIndex>
     </MissingIndexGroup>
    </MissingIndexes>
    <RelOp NodeId="0" PhysicalOp="Hash Match" LogicalOp="Aggregate" EstimateRows="2" EstimatedTotalSubtreeCost="0.0500">
     <Hash>
      <RelOp NodeId="1" PhysicalOp="Nested Loops" LogicalOp="Inner Join" EstimateRows="3" EstimatedTotalSubtreeCost="0.0300">
       <Warnings NoJoinPredicate="false"><PlanAffectingConvert ConvertIssue="Seek Plan" Expression="CONVERT_IMPLICIT(int,[o].[CustomerId],0)"/></Warnings>
       <NestedLoops Optimized="false">
        <RelOp NodeId="2" PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan" EstimateRows="3" EstimatedTotalSubtreeCost="0.0100">
         <IndexScan Ordered="false"><Object Database="[Shop]" Schema="[dbo]" Table="[Orders]" Index="[PK_Orders]" Alias="[o]"/></IndexScan>
        </RelOp>
        <RelOp NodeId="3" PhysicalOp="Clustered Index Seek" LogicalOp="Clustered Index Seek" EstimateRows="1" EstimatedTotalSubtreeCost="0.0150">
         <IndexScan Ordered="true"><Object Database="[Shop]" Schema="[dbo]" Table="[Customers]" Index="[PK_Customers]" Alias="[c]"/></IndexScan>
        </RelOp>
       </NestedLoops>
      </RelOp>
     </Hash>
    </RelOp>
   </QueryPlan>
  </StmtSimple>
 </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_operator_tree_costs_objects_and_missing_indexes() {
        let plan = parse(SAMPLE).unwrap();
        assert!(plan.statement.starts_with("SELECT c.Name"));
        let ops: Vec<_> = plan.nodes.iter().map(|n| (n.depth, n.physical.as_str(), n.object.as_deref())).collect();
        assert_eq!(ops, [(0, "Hash Match", None), (1, "Nested Loops", None), (2, "Clustered Index Scan", Some("[dbo].[Orders].[PK_Orders]")), (2, "Clustered Index Seek", Some("[dbo].[Customers].[PK_Customers]"))]);
        let pct: Vec<i64> = plan.nodes.iter().map(|n| n.cost_percent.round() as i64).collect();
        assert_eq!(pct, [40, 10, 20, 30]);
        assert_eq!(plan.nodes[1].warnings, ["PlanAffectingConvert: Seek Plan"]);
        assert_eq!(plan.missing_indexes, ["62%: [dbo].[Orders] ([CustomerId]) INCLUDE ([Total])"]);
        assert!(parse("<nope").is_err());
    }
}
