using System;
using System.Threading;

// Stands in for llama-server so the daemon's OWN spawn path creates a
// long-lived child while its HTTP listener is open. Whether that child inherits
// the listening socket is the thing under test.
//
// ⚠ It must IGNORE every argument and outlive the daemon. A stock binary that
// exits on unrecognised argv (ping, python, powershell were all tried) removes
// the child, and the test silently stops being able to fail.
public class PortReleaseSleeper
{
    public static void Main(string[] args)
    {
        Console.Error.WriteLine("port-release sleeper up, ignoring " + args.Length + " argument(s)");
        Thread.Sleep(300000);
    }
}
