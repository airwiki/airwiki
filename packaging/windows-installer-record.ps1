function Get-WindowsInstallerRecordStringData {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [object] $Record,

        [Parameter(Mandatory = $true)]
        [int] $Field
    )

    if ($Field -lt 1) {
        throw "Windows Installer record fields are one-based"
    }

    # Windows PowerShell 5.1 can misbind an indexed COM property inside a nested expression.
    # Invoke the documented property getter explicitly so the field index stays an integer.
    $Value = $Record.GetType().InvokeMember(
        "StringData",
        [System.Reflection.BindingFlags]::GetProperty,
        $null,
        $Record,
        [object[]]@($Field)
    )
    return [string] $Value
}
