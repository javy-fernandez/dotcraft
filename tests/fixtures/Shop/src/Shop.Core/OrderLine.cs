namespace Shop.Core;

/// <summary>One product in an order.</summary>
public sealed record OrderLine(string Sku, string Description, decimal UnitPrice, int Quantity)
{
    public decimal Subtotal => UnitPrice * Quantity;
}
