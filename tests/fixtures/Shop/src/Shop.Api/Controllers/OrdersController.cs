using Microsoft.AspNetCore.Mvc;
using Shop.Core;

namespace Shop.Api.Controllers;

public record QuoteLine(string Sku, string Description, decimal UnitPrice, int Quantity);

public record QuoteRequest(string Customer, string? Coupon, List<QuoteLine> Lines);

public record QuoteResponse(string Customer, int Items, decimal Subtotal, decimal Discount, decimal Tax, decimal Shipping, decimal Total);

[ApiController]
[Route("[controller]")]
public class OrdersController : ControllerBase
{
    private readonly PriceCalculator _calculator = new();

    /// <summary>Prices the sample order, optionally with a coupon.</summary>
    [HttpGet("sample")]
    public QuoteResponse Sample(string? coupon = null)
    {
        var order = Order.Sample();
        order.CouponCode = coupon;
        return Quote(order);
    }

    [HttpPost("quote")]
    public ActionResult<QuoteResponse> Post(QuoteRequest request)
    {
        try
        {
            var order = new Order(request.Customer) { CouponCode = request.Coupon };
            foreach (var l in request.Lines)
            {
                order.Add(l.Sku, l.Description, l.UnitPrice, l.Quantity);
            }
            return Quote(order);
        }
        catch (ArgumentException e)
        {
            return BadRequest(e.Message);
        }
    }

    private QuoteResponse Quote(Order order)
    {
        var p = _calculator.Calculate(order);
        return new QuoteResponse(order.Customer, order.ItemCount, p.Subtotal, p.Discount, p.Tax, p.Shipping, p.Total);
    }
}
