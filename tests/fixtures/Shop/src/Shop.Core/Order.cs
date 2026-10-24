namespace Shop.Core;

/// <summary>A customer order: lines plus an optional coupon.</summary>
public sealed class Order
{
    private readonly List<OrderLine> _lines = new();

    public Order(string customer)
    {
        if (string.IsNullOrWhiteSpace(customer))
        {
            throw new ArgumentException("A customer is required.", nameof(customer));
        }
        Customer = customer;
    }

    public string Customer { get; }

    public string? CouponCode { get; set; }

    public IReadOnlyList<OrderLine> Lines => _lines;

    public int ItemCount => _lines.Sum(l => l.Quantity);

    /// <summary>Adds a line; adding the same SKU again increases its quantity.</summary>
    public Order Add(string sku, string description, decimal unitPrice, int quantity = 1)
    {
        if (quantity <= 0)
        {
            throw new ArgumentOutOfRangeException(nameof(quantity), "Quantity must be positive.");
        }
        if (unitPrice < 0)
        {
            throw new ArgumentOutOfRangeException(nameof(unitPrice), "Price cannot be negative.");
        }

        var index = _lines.FindIndex(l => l.Sku == sku);
        if (index >= 0)
        {
            var existing = _lines[index];
            _lines[index] = existing with { Quantity = existing.Quantity + quantity };
        }
        else
        {
            _lines.Add(new OrderLine(sku, description, unitPrice, quantity));
        }
        return this;
    }

    /// <summary>The order used by the CLI and the API samples.</summary>
    public static Order Sample(string customer = "Ada") =>
        new Order(customer)
            .Add("KB-01", "Mechanical keyboard", 30.00m)
            .Add("MS-02", "Wireless mouse", 12.50m, 2);
}
