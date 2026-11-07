using System.Drawing;
using System.Threading;
using System.Windows.Forms;

namespace Kilobyt.App;

/// <summary>
/// Entry point. Offline by construction: no HTTP, no sockets, no update check (PRD §9).
/// </summary>
internal static class Program
{
    [STAThread]
    private static void Main()
    {
        // Equivalent to ApplicationConfiguration.Initialize(): per-monitor DPI v2 + Segoe UI.
        try
        {
            Application.SetHighDpiMode(HighDpiMode.PerMonitorV2);
        }
        catch (Exception)
        {
            // Older shells without DPI support: the manifest still requests PMv2.
        }

        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.SetDefaultFont(new Font("Segoe UI", 9F));

        // No crash dialog: report once, keep the process alive when it is only a UI fault.
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
        Application.ThreadException += (_, e) => ReportUnexpected(e.Exception, fatal: false);
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
            ReportUnexpected(e.ExceptionObject as Exception, fatal: e.IsTerminating);

        try
        {
            Application.Run(new MainForm());
        }
        catch (Exception ex)
        {
            ReportUnexpected(ex, fatal: true);
        }
    }

    private static void ReportUnexpected(Exception? ex, bool fatal)
    {
        try
        {
            string detail = ex?.Message ?? "Unknown error.";
            MessageBox.Show(
                fatal
                    ? $"Kilobyt hit an unexpected error and has to close.\n\n{detail}\n\nYour photos were not modified."
                    : $"Something went wrong, but Kilobyt is still running.\n\n{detail}",
                "Kilobyt",
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
        catch (Exception)
        {
            // Nothing sensible left to do if even the message box fails.
        }
    }
}
