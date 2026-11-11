using Humanizer;
using Newtonsoft.Json;

namespace CentralShop.Core;

public record Product(string Name, decimal Price)
{
    public string Title => Name.Humanize(LetterCasing.Title);

    public string ToJson() => JsonConvert.SerializeObject(this);
}
