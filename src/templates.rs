//! Template Browser contents and template parameters (`<name, type, default>`),
//! filled in with Ctrl+Shift+M as in SSMS.

pub struct Template {
    pub folder: &'static str,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template {
        folder: "Database",
        name: "Create Database",
        sql: "-- Create a database\nUSE master\nGO\n\nIF DB_ID(N'<Database_Name, sysname, SampleDB>') IS NULL\n\tCREATE DATABASE [<Database_Name, sysname, SampleDB>]\nGO\n",
    },
    Template {
        folder: "Database",
        name: "Drop Database",
        sql: "-- Drop a database, closing other connections first\nUSE master\nGO\n\nALTER DATABASE [<Database_Name, sysname, SampleDB>] SET SINGLE_USER WITH ROLLBACK IMMEDIATE\nGO\nDROP DATABASE [<Database_Name, sysname, SampleDB>]\nGO\n",
    },
    Template {
        folder: "Backup",
        name: "Backup Database",
        sql: "BACKUP DATABASE [<Database_Name, sysname, SampleDB>]\n\tTO DISK = N'<Backup_Path, nvarchar(4000), C:\\Backup\\SampleDB.bak>'\n\tWITH COMPRESSION, CHECKSUM, STATS = 10\nGO\n",
    },
    Template {
        folder: "Backup",
        name: "Restore Database",
        sql: "RESTORE DATABASE [<Database_Name, sysname, SampleDB>]\n\tFROM DISK = N'<Backup_Path, nvarchar(4000), C:\\Backup\\SampleDB.bak>'\n\tWITH REPLACE, RECOVERY, STATS = 10\nGO\n",
    },
    Template {
        folder: "Table",
        name: "Create Table",
        sql: "USE [<Database_Name, sysname, SampleDB>]\nGO\n\nCREATE TABLE [<Schema_Name, sysname, dbo>].[<Table_Name, sysname, SampleTable>]\n(\n\t[<Column1_Name, sysname, Id>] int IDENTITY(1,1) NOT NULL,\n\t[<Column2_Name, sysname, Name>] nvarchar(100) NULL,\n\tCONSTRAINT [PK_<Table_Name, sysname, SampleTable>] PRIMARY KEY CLUSTERED ([<Column1_Name, sysname, Id>])\n)\nGO\n",
    },
    Template {
        folder: "Table",
        name: "Add Column",
        sql: "ALTER TABLE [<Schema_Name, sysname, dbo>].[<Table_Name, sysname, SampleTable>]\n\tADD [<Column_Name, sysname, NewColumn>] <Data_Type, , int> NULL\nGO\n",
    },
    Template {
        folder: "Index",
        name: "Create Index",
        sql: "CREATE NONCLUSTERED INDEX [<Index_Name, sysname, IX_SampleTable_Name>]\n\tON [<Schema_Name, sysname, dbo>].[<Table_Name, sysname, SampleTable>] ([<Column_Name, sysname, Name>])\n\tINCLUDE ([<Included_Column, sysname, Id>])\nGO\n",
    },
    Template {
        folder: "Security",
        name: "Create SQL Login",
        sql: "CREATE LOGIN [<Login_Name, sysname, app_user>]\n\tWITH PASSWORD = N'<Password, sysname, ChangeMe!123>',\n\tDEFAULT_DATABASE = [<Database_Name, sysname, master>],\n\tCHECK_POLICY = ON\nGO\n",
    },
    Template {
        folder: "Security",
        name: "Create Windows Login (Active Directory)",
        sql: "-- An Active Directory user or group\nCREATE LOGIN [<Domain, sysname, CORP>\\<Account, sysname, SQL Admins>] FROM WINDOWS\n\tWITH DEFAULT_DATABASE = [<Database_Name, sysname, master>]\nGO\n",
    },
    Template {
        folder: "Security",
        name: "Create User for Login",
        sql: "USE [<Database_Name, sysname, SampleDB>]\nGO\n\nCREATE USER [<User_Name, sysname, app_user>] FOR LOGIN [<Login_Name, sysname, app_user>]\nGO\nALTER ROLE [<Role_Name, sysname, db_datareader>] ADD MEMBER [<User_Name, sysname, app_user>]\nGO\n",
    },
    Template {
        folder: "Stored Procedure",
        name: "Create Procedure",
        sql: "CREATE OR ALTER PROCEDURE [<Schema_Name, sysname, dbo>].[<Procedure_Name, sysname, usp_Sample>]\n\t@<Param1, sysname, p1> <Param1_Type, , int> = <Param1_Default, , 0>\nAS\nBEGIN\n\tSET NOCOUNT ON;\n\n\tSELECT @<Param1, sysname, p1>;\nEND\nGO\n",
    },
    Template {
        folder: "View",
        name: "Create View",
        sql: "CREATE OR ALTER VIEW [<Schema_Name, sysname, dbo>].[<View_Name, sysname, vSample>]\nAS\nSELECT <Columns, , *>\nFROM [<Schema_Name, sysname, dbo>].[<Table_Name, sysname, SampleTable>]\nGO\n",
    },
    Template {
        folder: "Monitoring",
        name: "Running Requests",
        sql: "SELECT r.session_id, s.login_name, s.host_name, DB_NAME(r.database_id) AS database_name,\n\tr.status, r.command, r.wait_type, r.wait_time, r.blocking_session_id,\n\tr.cpu_time, r.total_elapsed_time, t.text AS sql_text\nFROM sys.dm_exec_requests r\nJOIN sys.dm_exec_sessions s ON s.session_id = r.session_id\nCROSS APPLY sys.dm_exec_sql_text(r.sql_handle) t\nWHERE r.session_id <> @@SPID\nORDER BY r.total_elapsed_time DESC\n",
    },
    Template {
        folder: "Monitoring",
        name: "Database File Sizes",
        sql: "SELECT DB_NAME(database_id) AS database_name, name AS file_name, type_desc,\n\tCAST(size * 8 / 1024.0 AS decimal(12,2)) AS size_mb, physical_name\nFROM sys.master_files\nORDER BY database_name, type_desc\n",
    },
];

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: String,
    pub value: String,
}

/// The `<name, type, value>` placeholders in `sql`, each name once, in order.
pub fn params(sql: &str) -> Vec<Param> {
    let mut out: Vec<Param> = Vec::new();
    let mut rest = sql;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else { break };
        let inner = &after[..close];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 && !inner.contains('\n') && !parts[0].trim().is_empty() {
            let name = parts[0].trim().to_string();
            if !out.iter().any(|p| p.name == name) {
                out.push(Param { name, ty: parts[1].trim().to_string(), value: parts[2].trim().to_string() });
            }
            rest = &after[close + 1..];
        } else {
            rest = after;
        }
    }
    out
}

/// Replaces each placeholder with the value given for its name.
pub fn fill(sql: &str, values: &[Param]) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let replaced = after.find('>').and_then(|close| {
            let inner = &after[..close];
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() != 3 || inner.contains('\n') {
                return None;
            }
            let p = values.iter().find(|p| p.name == parts[0].trim())?;
            Some((p.value.clone(), close))
        });
        match replaced {
            Some((value, close)) => {
                out.push_str(&value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters() {
        let sql = "CREATE TABLE <T, sysname, x> (a int) -- <T, sysname, x> where a < 5 and b > 2";
        let p = params(sql);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0], Param { name: "T".into(), ty: "sysname".into(), value: "x".into() });
        let filled = fill(sql, &[Param { name: "T".into(), ty: String::new(), value: "Orders".into() }]);
        assert_eq!(filled, "CREATE TABLE Orders (a int) -- Orders where a < 5 and b > 2");
    }

    #[test]
    fn all_templates_parse() {
        for t in TEMPLATES {
            assert!(!params(t.sql).is_empty() || t.folder == "Monitoring", "{}", t.name);
        }
    }
}
