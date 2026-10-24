namespace Shop.Core;

/// <summary>The price of an order, step by step.</summary>
public sealed record PriceBreakdown(decimal Subtotal, decimal Discount, decimal Tax, decimal Shipping)
{
    public decimal Total => Subtotal - Discount + Tax + Shipping;
}

/// <summary>Prices orders: coupons, bulk discount, VAT and shipping.</summary>
public class PriceCalculator
{
    public const decimal TaxRate = 0.21m;
    public const decimal FreeShippingThreshold = 50m;
    public const decimal ShippingCost = 4.95m;
    public const int BulkItems = 10;

    public PriceBreakdown Calculate(Order order)
    {
        ArgumentNullException.ThrowIfNull(order);

        var subtotal = 0m;
        foreach (var line in order.Lines)
        {
            subtotal += line.Subtotal; // Breakpoint here to watch the subtotal grow.
        }

        var discount = DiscountFor(order, subtotal);
        var taxable = subtotal - discount;
        var tax = Math.Round(taxable * TaxRate, 2, MidpointRounding.AwayFromZero);
        var shipping = order.Lines.Count == 0 || taxable >= FreeShippingThreshold ? 0m : ShippingCost;
        return new PriceBreakdown(subtotal, discount, tax, shipping);
    }

    /// <summary>SAVE10 takes 10%, FIVEOFF takes 5 from 20 up; otherwise 5% from 10 items.</summary>
    public static decimal DiscountFor(Order order, decimal subtotal)
    {
        switch (order.CouponCode?.Trim().ToUpperInvariant())
        {
            case "SAVE10":
                return Math.Round(subtotal * 0.10m, 2, MidpointRounding.AwayFromZero);
            case "FIVEOFF" when subtotal >= 20m:
                return 5m;
            default:
                return order.ItemCount >= BulkItems ? Math.Round(subtotal * 0.05m, 2, MidpointRounding.AwayFromZero) : 0m;
        }
    }
}
