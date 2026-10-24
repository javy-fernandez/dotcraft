//! Generated C# source for benchmarks and performance tests.

/// A realistic C# file of about `lines` lines: classes with fields, properties, methods,
/// LINQ, string interpolation, comments and regions.
pub fn big_csharp(lines: usize) -> String {
    let mut s = String::from("using System;\nusing System.Collections.Generic;\nusing System.Linq;\nusing System.Threading.Tasks;\n\nnamespace DotCraft.Samples.Large;\n\n");
    let mut i = 0;
    while s.lines().count() < lines {
        s.push_str(&format!(
            r#"/// <summary>Service number {i}, handles orders.</summary>
public sealed class OrderService{i} : IOrderService, IDisposable
{{
    private readonly Dictionary<int, string> _cache = new();
    private const int MaxItems = {i};
    public string Name {{ get; set; }} = "Service {i}";

    #region Processing
    public async Task<int> ProcessAsync(IEnumerable<int> items, string label)
    {{
        // Filter and sum the even items.
        var total = items.Where(x => x % 2 == 0).Select(x => x * {i}).Sum();
        if (total > MaxItems && label != null)
        {{
            Console.WriteLine($"{{label}}: total {{total}} exceeds {{MaxItems}}\n");
            await Task.Delay(10);
        }}
        else
        {{
            _cache[total] = label ?? string.Empty;
        }}
        for (int k = 0; k < 3; k++)
        {{
            total += k switch {{ 0 => 1, 1 => 2, _ => 3 }};
        }}
        return total;
    }}
    #endregion

    public void Dispose() => _cache.Clear();
}}

"#
        ));
        i += 1;
    }
    s
}
