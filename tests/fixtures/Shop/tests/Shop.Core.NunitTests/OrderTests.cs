namespace Shop.Core.NunitTests;

public class OrderTests
{
    [Test]
    public void AddingTheSameSkuMergesLines()
    {
        var order = new Order("Ada").Add("KB-01", "Keyboard", 30m).Add("KB-01", "Keyboard", 30m, 2);

        Assert.That(order.Lines, Has.Count.EqualTo(1));
        Assert.That(order.ItemCount, Is.EqualTo(3));
    }

    [Test]
    public void QuantityMustBePositive()
    {
        var order = new Order("Ada");

        Assert.Throws<ArgumentOutOfRangeException>(() => order.Add("KB-01", "Keyboard", 30m, 0));
    }

    [TestCase(9, 0)]
    [TestCase(10, 5)]
    public void BulkOrdersGetFivePercent(int quantity, int expectedDiscount)
    {
        var order = new Order("Ada").Add("PEN", "Pen", 10m, quantity);

        Assert.That(PriceCalculator.DiscountFor(order, quantity * 10m), Is.EqualTo((decimal)expectedDiscount));
    }
}
