using Godot;

public partial class Main : Node
{
    public override void _Ready()
    {
        GD.Print("DOTCRAFT_GODOT_READY");
        GetTree().Quit();
    }
}
