// Biorouter Copilot's WinForms fixture app: the app the Windows acceptance drives (AD-21).
//
// It is the app under test, not the test. crates/biorouter-mcp/tests/computer_use_fixture.rs
// starts it, drives it through Copilot's own tools and reads every outcome from the state file
// this app writes, never from a tool's own report. The Linux twin is
// scripts/copilot-fixture-gtk.py; both keep the same names, titles and state keys.
//
// It is compiled to its own executable (never hosted in PowerShell, which Copilot treats as a
// terminal and refuses to type into) by Windows PowerShell 5.1's C# compiler, so it stays C# 5:
//
//   powershell -NoProfile -Command "Add-Type -OutputType WindowsApplication `
//     -OutputAssembly target\copilot-fixture\BiorouterCopilotFixture.exe `
//     -ReferencedAssemblies System.Windows.Forms,System.Drawing `
//     -TypeDefinition (Get-Content -Raw scripts\copilot-fixture-winforms.cs)"
//
// Usage: BiorouterCopilotFixture.exe WORK_DIR
//
// State keys in WORK_DIR\state.json: input, applied, checked, secret_length (never the text),
// order, dropped, scroll, pointer_pressed, console_windows (the most console windows seen at
// once beyond those open at start, so a helper that flashes a console shows), pid and app (the
// process name). WORK_DIR\ready appears once both windows are shown.
using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Forms;

public static class BiorouterCopilotFixture
{
    const string Title = "Biorouter Copilot Fixture";
    const string DropTitle = "Biorouter Copilot Fixture Drop";
    const string DragText = "Dropped by Copilot";
    const string RowPrefix = "biorouter-fixture-row:";

    static string work;
    static string input = "", applied = "", dropped = "";
    static bool optionChecked, pointerPressed;
    static int secretLength, scroll, consoleWindows;
    static List<string> order = new List<string>(new string[] { "Alpha", "Beta", "Gamma" });

    delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);

    static string Quote(string value)
    {
        var builder = new StringBuilder("\"");
        foreach (char c in value)
        {
            if (c == '"' || c == '\\') builder.Append('\\').Append(c);
            else if (c < ' ') builder.AppendFormat("\\u{0:x4}", (int)c);
            else builder.Append(c);
        }
        return builder.Append('"').ToString();
    }

    static void Save()
    {
        var items = new List<string>();
        foreach (string item in order) items.Add(Quote(item));
        var self = System.Diagnostics.Process.GetCurrentProcess();
        string json = "{\"pid\":" + self.Id + ",\"app\":" + Quote(self.ProcessName)
            + ",\"input\":" + Quote(input) + ",\"applied\":" + Quote(applied)
            + ",\"checked\":" + (optionChecked ? "true" : "false")
            + ",\"secret_length\":" + secretLength
            + ",\"order\":[" + string.Join(",", items.ToArray()) + "]"
            + ",\"dropped\":" + Quote(dropped) + ",\"scroll\":" + scroll
            + ",\"pointer_pressed\":" + (pointerPressed ? "true" : "false")
            + ",\"console_windows\":" + consoleWindows + "}";
        string temporary = Path.Combine(work, "state.json.tmp");
        File.WriteAllText(temporary, json);
        string final = Path.Combine(work, "state.json");
        if (File.Exists(final)) File.Replace(temporary, final, null);
        else File.Move(temporary, final);
    }

    static int VisibleConsoles()
    {
        int count = 0;
        EnumWindows(delegate (IntPtr window, IntPtr parameter)
        {
            if (!IsWindowVisible(window)) return true;
            var name = new StringBuilder(256);
            GetClassName(window, name, name.Capacity);
            string kind = name.ToString();
            if (kind == "ConsoleWindowClass" || kind == "CASCADIA_HOSTING_WINDOW_CLASS") count++;
            return true;
        }, IntPtr.Zero);
        return count;
    }

    [STAThread]
    public static void Main(string[] args)
    {
        if (args.Length != 1) { Console.Error.WriteLine("usage: BiorouterCopilotFixture WORK_DIR"); return; }
        work = args[0];
        Application.EnableVisualStyles();

        var form = new Form();
        form.Text = Title;
        form.StartPosition = FormStartPosition.Manual;
        form.Location = new Point(60, 60);
        form.Size = new Size(520, 640);

        var field = new TextBox();
        field.Name = "FixtureInput"; field.AccessibleName = "FixtureInput";
        field.SetBounds(20, 20, 460, 24);
        field.TextChanged += delegate { input = field.Text; Save(); };

        var apply = new Button();
        apply.Text = "Apply fixture"; apply.AccessibleName = "Apply fixture";
        apply.SetBounds(20, 56, 160, 30);
        apply.Click += delegate { applied = field.Text; Save(); };

        var option = new CheckBox();
        option.Text = "Fixture option"; option.AccessibleName = "Fixture option";
        option.SetBounds(20, 96, 200, 24);
        option.CheckedChanged += delegate { optionChecked = option.Checked; Save(); };

        var secret = new TextBox();
        secret.Name = "FixtureSecret"; secret.AccessibleName = "FixtureSecret";
        secret.UseSystemPasswordChar = true;
        secret.SetBounds(20, 130, 460, 24);
        secret.TextChanged += delegate { secretLength = secret.Text.Length; Save(); };

        // A list reordered by dragging one item onto another: dropping X anywhere on Y puts X
        // where Y was, so the test can assert the order exactly.
        var list = new ListBox();
        list.Name = "FixtureList"; list.AccessibleName = "FixtureList";
        list.SetBounds(20, 166, 460, 90);
        list.Items.AddRange(order.ToArray());
        list.AllowDrop = true;
        Point pressAt = Point.Empty;
        int pressed = -1;
        list.MouseDown += delegate (object sender, MouseEventArgs e)
        {
            pressed = e.Button == MouseButtons.Left ? list.IndexFromPoint(e.Location) : -1;
            pressAt = e.Location;
        };
        list.MouseMove += delegate (object sender, MouseEventArgs e)
        {
            if (pressed < 0 || (e.Button & MouseButtons.Left) == 0) return;
            Size slop = SystemInformation.DragSize;
            if (Math.Abs(e.X - pressAt.X) < slop.Width && Math.Abs(e.Y - pressAt.Y) < slop.Height) return;
            string moved = (string)list.Items[pressed];
            pressed = -1;
            list.DoDragDrop(RowPrefix + moved, DragDropEffects.Move);
        };
        list.DragOver += delegate (object sender, DragEventArgs e)
        {
            string data = e.Data.GetData(DataFormats.UnicodeText) as string;
            e.Effect = data != null && data.StartsWith(RowPrefix) ? DragDropEffects.Move : DragDropEffects.None;
        };
        list.DragDrop += delegate (object sender, DragEventArgs e)
        {
            string data = e.Data.GetData(DataFormats.UnicodeText) as string;
            if (data == null || !data.StartsWith(RowPrefix)) return;
            string moved = data.Substring(RowPrefix.Length);
            int target = list.IndexFromPoint(list.PointToClient(new Point(e.X, e.Y)));
            if (target < 0 || !order.Contains(moved)) return;
            string onto = (string)list.Items[target];
            if (onto != moved)
            {
                order.Remove(moved);
                order.Insert(order.IndexOf(onto), moved);
                list.Items.Clear();
                list.Items.AddRange(order.ToArray());
            }
            Save();
        };

        // A button whose drag carries text to the other window's field.
        var source = new Button();
        source.Text = "Drag me"; source.AccessibleName = "Drag me";
        source.SetBounds(20, 266, 160, 30);
        Point sourceAt = Point.Empty;
        bool sourceDown = false;
        source.MouseDown += delegate (object sender, MouseEventArgs e)
        {
            sourceDown = e.Button == MouseButtons.Left; sourceAt = e.Location;
        };
        source.MouseMove += delegate (object sender, MouseEventArgs e)
        {
            if (!sourceDown || (e.Button & MouseButtons.Left) == 0) return;
            Size slop = SystemInformation.DragSize;
            if (Math.Abs(e.X - sourceAt.X) < slop.Width && Math.Abs(e.Y - sourceAt.Y) < slop.Height) return;
            sourceDown = false;
            source.DoDragDrop(DragText, DragDropEffects.Copy);
        };

        var scroller = new TextBox();
        scroller.Name = "FixtureScroll"; scroller.AccessibleName = "FixtureScroll";
        scroller.Multiline = true; scroller.ReadOnly = true; scroller.ScrollBars = ScrollBars.Vertical;
        var lines = new StringBuilder();
        for (int i = 0; i < 400; i++) lines.AppendFormat("Fixture scroll row {0:0000}\r\n", i);
        scroller.Text = lines.ToString();
        scroller.SetBounds(20, 306, 460, 260);

        form.Controls.AddRange(new Control[] { field, apply, option, secret, list, source, scroller });

        var dropForm = new Form();
        dropForm.Text = DropTitle;
        dropForm.StartPosition = FormStartPosition.Manual;
        dropForm.Location = new Point(640, 60);
        dropForm.Size = new Size(360, 140);
        var drop = new TextBox();
        drop.Name = "FixtureDrop"; drop.AccessibleName = "FixtureDrop";
        drop.SetBounds(20, 20, 300, 24);
        drop.AllowDrop = true;
        drop.DragEnter += delegate (object sender, DragEventArgs e)
        {
            string data = e.Data.GetData(DataFormats.UnicodeText) as string;
            e.Effect = data != null && !data.StartsWith(RowPrefix) ? DragDropEffects.Copy : DragDropEffects.None;
        };
        drop.DragDrop += delegate (object sender, DragEventArgs e)
        {
            string data = e.Data.GetData(DataFormats.UnicodeText) as string;
            if (data != null) drop.Text = data;
        };
        drop.TextChanged += delegate { dropped = drop.Text; Save(); };
        dropForm.Controls.Add(drop);

        // The scroll position, the left button (a press the release guard failed to lift would
        // stay down) and the most console windows visible at once, read every 100 ms.
        int baseline = -1;
        var timer = new Timer();
        timer.Interval = 100;
        timer.Tick += delegate
        {
            bool changed = false;
            const int EM_GETFIRSTVISIBLELINE = 0x00CE;
            int first = (int)SendMessage(scroller.Handle, EM_GETFIRSTVISIBLELINE, IntPtr.Zero, IntPtr.Zero);
            if (first != scroll) { scroll = first; changed = true; }
            bool down = (Control.MouseButtons & MouseButtons.Left) != 0;
            if (down != pointerPressed) { pointerPressed = down; changed = true; }
            int consoles = VisibleConsoles();
            if (baseline < 0) baseline = consoles;
            if (consoles - baseline > consoleWindows) { consoleWindows = consoles - baseline; changed = true; }
            if (changed) Save();
        };

        form.Shown += delegate
        {
            dropForm.Show();
            form.Activate();
            Save();
            timer.Start();
            File.WriteAllText(Path.Combine(work, "ready"), "ready");
        };
        Save();
        Application.Run(form);
        timer.Dispose();
    }

    [DllImport("user32.dll")]
    static extern IntPtr SendMessage(IntPtr window, int message, IntPtr wParam, IntPtr lParam);
}
