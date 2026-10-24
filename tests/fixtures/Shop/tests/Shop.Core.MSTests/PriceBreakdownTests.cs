namespace Shop.Core.MSTests;

[TestClass]
public sealed class PriceBreakdownTests
{
    [TestMethod]
    public void TotalAddsTaxAndShipping()
    {
        var breakdown = new PriceBreakdown(Subtotal: 40m, Discount: 0m, Tax: 8.40m, Shipping: 4.95m);

        Assert.AreEqual(53.35m, breakdown.Total);
    }

    [TestMethod]
    public void TaxIsRoundedAwayFromZero()
    {
        var order = Order.Sample();
        order.CouponCode = "SAVE10";

        var price = new PriceCalculator().Calculate(order);

        Assert.AreEqual(10.40m, price.Tax);
        Assert.AreEqual(4.95m, price.Shipping);
        Assert.AreEqual(64.85m, price.Total);
    }

    [TestMethod]
    [DataRow("Ada")]
    [DataRow("Grace")]
    public void SampleKeepsTheCustomer(string customer)
    {
        Assert.AreEqual(customer, Order.Sample(customer).Customer);
    }
}
