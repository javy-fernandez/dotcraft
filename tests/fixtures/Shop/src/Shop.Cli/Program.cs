using System.Globalization;
using Shop.Core;

// Usage: Shop.Cli [customer] [--coupon CODE] [--exit N]
var customer = "Ada";
string? coupon = null;
var exitCode = 0;
for (var i = 0; i < args.Length; i++)
{
    switch (args[i])
    {
        case "--coupon" when i + 1 < args.Length:
            coupon = args[++i];
            break;
        case "--exit" when i + 1 < args.Length:
            exitCode = int.Parse(args[++i], CultureInfo.InvariantCulture);
            break;
        default:
            customer = args[i];
            break;
    }
}

var order = Order.Sample(customer);
order.CouponCode = coupon;
var price = new PriceCalculator().Calculate(order);

var inv = CultureInfo.InvariantCulture;
Console.WriteLine($"Order for {order.Customer} ({order.ItemCount} items)");
foreach (var line in order.Lines)
{
    Console.WriteLine(string.Format(inv, "  {0,-6} {1,-20} {2,3} x {3,7:0.00} = {4,7:0.00}", line.Sku, line.Description, line.Quantity, line.UnitPrice, line.Subtotal));
}
Console.WriteLine(string.Format(inv, "Subtotal: {0:0.00}", price.Subtotal));
Console.WriteLine(string.Format(inv, "Discount: {0:0.00}", price.Discount));
Console.WriteLine(string.Format(inv, "Tax: {0:0.00}", price.Tax));
Console.WriteLine(string.Format(inv, "Shipping: {0:0.00}", price.Shipping));
Console.WriteLine(string.Format(inv, "Total: {0:0.00}", price.Total));
Console.Error.WriteLine("done");
return exitCode;
