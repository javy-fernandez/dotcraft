namespace Shop.Core.XunitTests;

public class PriceCalculatorTests
{
    private readonly PriceCalculator _calculator = new();

    [Fact]
    public void SampleOrderGetsFreeShipping()
    {
        var price = _calculator.Calculate(Order.Sample());

        Assert.Equal(55.00m, price.Subtotal);
        Assert.Equal(0m, price.Shipping);
        Assert.Equal(66.55m, price.Total);
    }

    [Fact]
    public void EmptyOrderCostsNothing()
    {
        var price = _calculator.Calculate(new Order("Bob"));

        Assert.Equal(0m, price.Total);
    }

    [Theory]
    [InlineData("SAVE10", 5.50)]
    [InlineData("fiveoff", 5.00)]
    [InlineData(null, 0.00)]
    public void CouponsGiveDiscounts(string? coupon, double expected)
    {
        var order = Order.Sample();
        order.CouponCode = coupon;

        Assert.Equal((decimal)expected, _calculator.Calculate(order).Discount);
    }
}
